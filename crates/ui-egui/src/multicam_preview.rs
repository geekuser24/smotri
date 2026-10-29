//! Content revisions for nested camera sources. Editing the outer cut must not invalidate
//! the camera grid. Immutable sequence snapshots also let us avoid serializing keyframes
//! again on every UI repaint (or on an unrelated edit).

use std::collections::{BTreeSet, HashMap};
use std::hash::{DefaultHasher, Hash, Hasher};
use std::io::{self, Write};
use std::sync::{Arc, Weak};

use filmcraft_project::{ItemId, ItemKind, Project, Sequence};

#[derive(Default)]
pub(crate) struct Revisions {
    project: Weak<Project>,
    proxies: bool,
    roots: HashMap<ItemId, u64>,
    sequences: HashMap<ItemId, (Weak<Sequence>, u64)>,
}

struct HashWriter(DefaultHasher);

impl Write for HashWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0.write(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Revisions {
    pub(crate) fn get(&mut self, project: &Arc<Project>, root: ItemId, proxies: bool) -> Result<u64, String> {
        if !self.project.ptr_eq(&Arc::downgrade(project)) || self.proxies != proxies {
            self.project = Arc::downgrade(project);
            self.proxies = proxies;
            self.roots.clear();
            self.sequences.retain(|id, _| project.items.contains_key(id));
        }
        if let Some(revision) = self.roots.get(&root) {
            return Ok(*revision);
        }
        let mut hash = HashWriter(DefaultHasher::new());
        proxies.hash(&mut hash.0);
        serde_json::to_writer(&mut hash, &project.settings).map_err(|e| e.to_string())?;
        let mut pending = BTreeSet::from([root]);
        let mut seen = BTreeSet::new();
        while let Some(id) = pending.pop_first() {
            if !seen.insert(id) {
                continue;
            }
            // Iterative traversal, including malformed/cyclic projects, with bounded work.
            if seen.len() > 100_000 {
                return Err("too many nested camera sources".into());
            }
            id.hash(&mut hash.0);
            let Some(item) = project.item(id) else { continue };
            match &item.kind {
                ItemKind::Sequence(sequence) => {
                    let revision = match self.sequences.get(&id) {
                        Some((snapshot, revision)) if snapshot.ptr_eq(&Arc::downgrade(sequence)) => *revision,
                        _ => {
                            let mut own = HashWriter(DefaultHasher::new());
                            serde_json::to_writer(&mut own, sequence).map_err(|e| e.to_string())?;
                            let revision = own.0.finish();
                            self.sequences.insert(id, (Arc::downgrade(sequence), revision));
                            revision
                        }
                    };
                    revision.hash(&mut hash.0);
                    for track in &sequence.video_tracks {
                        for clip in &track.items {
                            pending.insert(clip.item);
                            if pending.len() > 100_000 {
                                return Err("too many nested camera sources".into());
                            }
                        }
                    }
                }
                kind => {
                    serde_json::to_writer(&mut hash, kind).map_err(|e| e.to_string())?;
                    if let ItemKind::Subclip { parent, .. } = kind {
                        pending.insert(*parent);
                    }
                }
            }
        }
        let revision = hash.0.finish();
        self.roots.insert(root, revision);
        Ok(revision)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use filmcraft_project::{SequenceSettings, TrackKind};
    use filmcraft_time::{FrameRate, Tick, TimeRange};

    fn nested_project() -> (Arc<Project>, ItemId, ItemId, ItemId) {
        let mut project = Project::new("camera cache");
        let camera = project.new_sequence("camera", SequenceSettings::default(), 1, 0, None);
        let source = project.new_sequence("source", SequenceSettings::default(), 1, 0, None);
        let outer = project.new_sequence("edit", SequenceSettings::default(), 1, 0, None);
        for (parent, child) in [(source, camera), (outer, source)] {
            let clip = project
                .make_track_item(child, TrackKind::Video, Tick::ZERO, TimeRange::new(Tick::ZERO, FrameRate::FPS_24.tick_of(48)), FrameRate::FPS_24)
                .unwrap();
            project.sequence_mut(parent).unwrap().video_tracks[0].items.push(clip);
        }
        (Arc::new(project), source, camera, outer)
    }

    #[test]
    fn outer_edits_reuse_nested_camera_frames_but_source_edits_invalidate() {
        let (mut project, source, camera, outer) = nested_project();
        let mut revisions = Revisions::default();
        let original = revisions.get(&project, source, false).unwrap();
        assert_eq!(revisions.get(&project, source, false).unwrap(), original);
        Arc::make_mut(&mut project).sequence_mut(outer).unwrap().video_tracks[0].items[0].source_in = FrameRate::FPS_24.tick_of(1);
        assert_eq!(revisions.get(&project, source, false).unwrap(), original, "outer trim/cut must keep camera frames");
        let before = project.clone();
        Arc::make_mut(&mut project).sequence_mut(camera).unwrap().settings.width = 640;
        assert_ne!(revisions.get(&project, source, false).unwrap(), original, "nested source edit must invalidate");
        assert_eq!(revisions.get(&before, source, false).unwrap(), original, "Undo restores the content revision");
        assert_ne!(revisions.get(&before, source, true).unwrap(), original, "proxy mode has its own frames");
    }

    #[test]
    fn cyclic_and_missing_camera_sources_are_bounded() {
        let (mut project, source, camera, _) = nested_project();
        let back = project.sequence(source).unwrap().video_tracks[0].items[0].clone();
        let p = Arc::make_mut(&mut project);
        let q = p.sequence_mut(camera).unwrap();
        q.video_tracks[0].items.push(filmcraft_project::TrackItem { item: source, ..back });
        let mut revisions = Revisions::default();
        assert!(revisions.get(&project, source, false).is_ok());
        assert!(revisions.get(&project, ItemId(u64::MAX), false).is_ok());
    }
}
