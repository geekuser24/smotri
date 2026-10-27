use super::*;
use crate::media_test_util::tmp_dir;
use filmcraft_project::*;
use filmcraft_time::{FrameRate, Tick, TimeRange};

fn fixture() -> (Project, ItemId, ItemId) {
    let (mut p, seq) = crate::demo::demo_project(&crate::MediaPool::default());
    let child = p.new_sequence("Nested", SequenceSettings::default(), 1, 1, None);
    let media = p.items.values().find(|i| matches!(i.kind, ItemKind::Media(_))).unwrap().id;
    let c = p.make_track_item(media, TrackKind::Video, Tick::ZERO, TimeRange::new(Tick::ZERO, FrameRate::FPS_24.tick_of(24)), FrameRate::FPS_24).unwrap();
    p.sequence_mut(child).unwrap().video_tracks[0].items.push(c);
    let nest = p.make_track_item(child, TrackKind::Video, Tick::ZERO, TimeRange::new(Tick::ZERO, FrameRate::FPS_24.tick_of(24)), FrameRate::FPS_24).unwrap();
    p.sequence_mut(seq).unwrap().video_tracks[0].items.push(nest);
    p.transcripts.insert(media, Arc::new(Transcript { language: "ru".into(), ..Default::default() }));
    p.source_graphics.insert(media, SourceGraphic::default());
    p.notes = "Source notes".into();
    (p, seq, child)
}
fn encoded(p: &Project) -> Vec<u8> {
    filmcraft_format::encode(p, false)
}
#[test]
fn self_import_is_independent_preserves_destination_and_undo_redo() {
    let (p, seq, child) = fixture();
    let dir = tmp_dir("native-import");
    let path = dir.join("source.fcproj");
    std::fs::write(&path, encoded(&p)).unwrap();
    let mut s = Session::default();
    s.execute("file.open", json!({"path":path})).unwrap();
    let before = encoded(&s.project);
    let state = s.state.clone();
    let original_path = s.path.clone();
    let original_settings = s.project.settings.clone();
    let max = s.project.max_id();
    let r = s.execute("file.importProject", json!({"path":path})).unwrap();
    let offset = max + 1;
    assert_eq!(s.path, original_path);
    assert_eq!(s.project.settings, original_settings);
    assert_eq!(s.state.active_sequence, state.active_sequence);
    assert_eq!(s.project.items.len(), p.items.len() * 2);
    assert!(s.project.sequence(ItemId(seq.0 + offset)).unwrap().video_tracks[0].items.iter().any(|c| c.item == ItemId(child.0 + offset)));
    for (id, item) in &p.items {
        assert_eq!(serde_json::to_value(s.project.item(*id).unwrap()).unwrap(), serde_json::to_value(item).unwrap());
    }
    assert_eq!(s.project.transcripts.len(), 2 * p.transcripts.len());
    assert_eq!(s.project.source_graphics.len(), 2 * p.source_graphics.len());
    let bin = BinId(r["bin"].as_u64().unwrap());
    assert!(s.project.root.find_bin(bin).is_some());
    let after = encoded(&s.project);
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(encoded(&s.project), before);
    s.execute("edit.redo", json!({})).unwrap();
    assert_eq!(encoded(&s.project), after);
    let loaded = filmcraft_format::decode(&after).unwrap();
    assert_eq!(encoded(&loaded.project), after);
    s.execute("file.import", json!({"paths":[path]})).unwrap();
    assert_eq!(s.project.items.len(), p.items.len() * 3);
    std::fs::remove_dir_all(dir).unwrap();
}
#[test]
fn full_id_graph_and_lut_references_are_remapped() {
    let (mut src, seq, child) = fixture();
    src.luts.push(ProjectLut {
        id: "lut1".into(),
        name: "Source LUT".into(),
        format: "cube".into(),
        text: "LUT_1D_SIZE 2\n0 0 0\n1 1 1".into(),
        source_path: None,
    });
    let mut fx = find_effect("lumetri").unwrap().instance();
    fx.params.get_mut("input_lut").unwrap().value = ParamValue::Text("lib:lut1".into());
    fx.params.get_mut("look_lut").unwrap().put_keyframe(Tick::ZERO, ParamValue::Text("lib:lut1".into()));
    let submix = TrackId(src.alloc_id());
    let q = src.sequence_mut(seq).unwrap();
    q.submix_tracks.push(Track::new(submix, TrackKind::Audio, "Bus".into()));
    q.audio_tracks[0].mixer.output = Some(submix);
    q.audio_tracks[0].mixer.sends.push(TrackSend::new(submix));
    q.video_tracks[0].items[0].effects.push(fx.clone());
    q.master_effects.push(fx);
    q.multicam = Some(MulticamSource {
        cameras: vec![Camera {
            name: "Angle".into(),
            video_track: Some(q.video_tracks[0].id),
            audio_tracks: vec![q.audio_tracks[0].id],
            enabled: true,
            source: Some(child),
        }],
        ..Default::default()
    });
    q.merged = Some(MergedClip { video: Some(child), audio: vec![child], sync: "audio".into() });
    let mut dst = src.clone();
    let offset = dst.max_id() + 1;
    let old = encoded(&dst);
    merge(&mut dst, src.clone(), None, "Imported").unwrap();
    let q = dst.sequence(ItemId(seq.0 + offset)).unwrap();
    let orig = src.sequence(seq).unwrap();
    assert_eq!(q.audio_tracks[0].mixer.output, Some(TrackId(submix.0 + offset)));
    assert_eq!(q.audio_tracks[0].mixer.sends[0].target, TrackId(submix.0 + offset));
    assert_eq!(q.multicam.as_ref().unwrap().cameras[0].video_track, Some(TrackId(orig.video_tracks[0].id.0 + offset)));
    assert_eq!(q.multicam.as_ref().unwrap().cameras[0].source, Some(ItemId(child.0 + offset)));
    assert_eq!(q.merged.as_ref().unwrap().video, Some(ItemId(child.0 + offset)));
    let new_lut = dst.luts.last().unwrap();
    assert_ne!(new_lut.id, "lut1");
    assert_eq!(q.master_effects.last().unwrap().param("input_lut").unwrap().value, ParamValue::Text(format!("lib:{}", new_lut.id)));
    assert_eq!(q.master_effects.last().unwrap().param("look_lut").unwrap().keyframes[0].value, ParamValue::Text(format!("lib:{}", new_lut.id)));
    assert!(dst.next_id > dst.max_id());
    assert_ne!(encoded(&dst), old);
}
#[test]
fn relative_media_and_proxies_keep_the_source_folder_after_save() {
    let (mut p, _, _) = fixture();
    let dir = tmp_dir("import-paths");
    let input = dir.join("input");
    let output = dir.join("output");
    std::fs::create_dir_all(&input).unwrap();
    std::fs::create_dir_all(&output).unwrap();
    let id = p.items.values().find(|i| i.as_media().is_some()).unwrap().id;
    let m = p.item_mut(id).unwrap().as_media_mut().unwrap();
    m.media = MediaRef::File { path: "clip.mov".into() };
    m.proxy = Some(MediaRef::File { path: "proxy.mov".into() });
    std::fs::write(input.join("clip.mov"), b"fixture").unwrap();
    let path = input.join("a.fcproj");
    std::fs::write(&path, encoded(&p)).unwrap();
    let mut s = Session { path: Some(output.join("dest.fcproj").to_string_lossy().into_owned()), ..Default::default() };
    let r = s.execute("file.import", json!({"paths":[path]})).unwrap();
    assert_eq!(r["errors"].as_array().unwrap().len(), 0);
    let item = s.project.items.values().find(|i| i.name == p.item(id).unwrap().name).unwrap();
    let m = item.as_media().unwrap();
    assert_eq!(m.media, MediaRef::File { path: input.join("clip.mov").to_string_lossy().into_owned() });
    assert_eq!(m.proxy, Some(MediaRef::File { path: input.join("proxy.mov").to_string_lossy().into_owned() }));
    assert_eq!(r["documents"][0]["missingMedia"].as_array().unwrap().len(), 1);
    let saved = filmcraft_format::decode(&encoded(&s.project)).unwrap();
    assert_eq!(saved.project.item(item.id).unwrap().as_media().unwrap().media, m.media);
    std::fs::remove_dir_all(dir).unwrap();
}
#[test]
fn hostile_inputs_are_atomic_and_never_panic() {
    let (p, _, _) = fixture();
    let dir = tmp_dir("import-hostile");
    let path = dir.join("bad.fcproj");
    let mut s = Session::default();
    let before = encoded(&s.project);
    for bytes in [b"broken".to_vec(), encoded(&p)[..100].to_vec(), br#"{"format":"filmcraft.project","schema_version":999999}"#.to_vec()] {
        std::fs::write(&path, bytes).unwrap();
        assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| s.execute("file.importProject", json!({"path":path})))).unwrap().is_err());
        assert_eq!(encoded(&s.project), before);
    }
    std::fs::write(&path, encoded(&p)).unwrap();
    for params in [json!({}), json!({"path":5}), json!({"path":path,"bin":999999})] {
        assert!(s.execute("file.importProject", params).is_err());
        assert_eq!(encoded(&s.project), before);
    }
    let mut huge = p.clone();
    huge.next_id = Project::MAX_NEXT_ID;
    let before = encoded(&huge);
    assert!(merge(&mut huge, p, None, "copy").is_err());
    assert_eq!(encoded(&huge), before);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn imported_nested_timeline_renders_identically_and_rejects_cycles() {
    let (mut p, seq, child) = fixture();
    let tid = TrackId(p.alloc_id());
    let cid = ClipId(p.alloc_id());
    let mut captions = CaptionTrack::new(tid, "Captions".into(), CaptionFormat::Subtitle);
    captions.captions.push(Caption {
        id: cid,
        start: Tick::ZERO,
        duration: FrameRate::FPS_24.tick_of(12),
        text: "Hello".into(),
        speaker: None,
        cue_id: None,
        settings: String::new(),
    });
    p.sequence_mut(child).unwrap().caption_tracks.push(captions);
    let media = p.items.values().find(|i| i.as_media().is_some()).unwrap().id;
    p.narrations.insert(
        media,
        Narration { text: "Hello".into(), language: "en".into(), voice: "default".into(), pitch: Default::default(), pace: 1., speech_duration: Tick::ZERO },
    );
    let dir = tmp_dir("import-render");
    let path = dir.join("source.fcproj");
    std::fs::write(&path, encoded(&p)).unwrap();
    let mut s = Session::default();
    s.execute("file.open", json!({"path":path})).unwrap();
    s.state.active_sequence = Some(seq);
    let time = FrameRate::FPS_24.tick_of(5);
    let before = s.render_program_at(0.125, time).unwrap();
    let offset = s.project.max_id() + 1;
    s.execute("file.importProject", json!({"path":path})).unwrap();
    s.state.active_sequence = Some(ItemId(seq.0 + offset));
    assert_eq!(s.render_program_at(0.125, time).unwrap(), before);
    assert_eq!(s.project.narrations.get(&ItemId(media.0 + offset)), p.narrations.get(&media));
    let c = &s.project.sequence(ItemId(child.0 + offset)).unwrap().caption_tracks[0];
    assert_eq!(c.id, TrackId(tid.0 + offset));
    assert_eq!(c.captions[0].id, ClipId(cid.0 + offset));
    let clip = p.make_track_item(seq, TrackKind::Video, Tick::ZERO, TimeRange::new(Tick::ZERO, FrameRate::FPS_24.tick_of(24)), FrameRate::FPS_24).unwrap();
    p.sequence_mut(child).unwrap().video_tracks[0].items.push(clip);
    std::fs::write(&path, encoded(&p)).unwrap();
    let before = encoded(&s.project);
    assert!(s.execute("file.importProject", json!({"path":path})).is_err());
    assert_eq!(encoded(&s.project), before);
    std::fs::remove_dir_all(dir).unwrap();
}
