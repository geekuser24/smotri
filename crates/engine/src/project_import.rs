//! Import a native project as an independent, undoable bin in the current document.
use crate::{EngineError, Result, Session};
use filmcraft_project::{Bin, BinEntry, BinId, EffectInstance, ItemId, ItemKind, MediaRef, MixerStrip, ParamValue, Project};
use serde_json::{Value, json};
use std::{collections::BTreeMap, path::Path, sync::Arc};

fn error(s: impl Into<String>) -> EngineError {
    EngineError::Other(s.into())
}
pub fn is_project(path: &str) -> bool {
    Path::new(path).extension().is_some_and(|e| e.eq_ignore_ascii_case("fcproj"))
}

pub fn import(s: &mut Session, path: &str, parent: Option<BinId>) -> Result<Value> {
    // Refuse excessive documents before reading on hosts that supply file sizes.
    const LIMIT: usize = 256 * 1024 * 1024;
    if s.services.file_size(path).is_ok_and(|n| n > LIMIT as u64) {
        return Err(error("Project exceeds the 256 MB import limit"));
    }
    let bytes = s.services.read_file(path).map_err(|e| error(format!("{path}: {e}")))?;
    if bytes.len() > LIMIT {
        return Err(error("Project exceeds the 256 MB import limit"));
    }
    let loaded = filmcraft_format::decode(&bytes).map_err(|e| error(format!("{path}: {e}")))?;
    let preferred = loaded.view.as_ref().and_then(|v| v.active_sequence);
    let mut fragment = loaded.project;
    let primary_index = fragment.items.values().filter(|i| i.as_sequence().is_some()).position(|i| Some(i.id) == preferred).unwrap_or(0);
    let mut missing = Vec::new();
    for item in fragment.items.values_mut() {
        if let ItemKind::Media(m) = &mut item.kind {
            for reference in std::iter::once(&mut m.media).chain(m.proxy.iter_mut()) {
                if let MediaRef::File { path: stored } = reference {
                    match crate::doc_path::resolve(stored, Some(Path::new(path))) {
                        Ok(absolute) => {
                            *stored = absolute;
                            if s.services.file_size(stored).is_err() {
                                missing.push(stored.clone());
                            }
                        }
                        Err(why) => {
                            missing.push(format!("{stored}: {why}"));
                        }
                    }
                }
            }
        }
    }
    missing.sort();
    missing.dedup();
    let name = if fragment.name.trim().is_empty() {
        Path::new(path).file_stem().and_then(|s| s.to_str()).unwrap_or("Imported project").to_string()
    } else {
        fragment.name.clone()
    };
    // Preserve the destination's active timeline, settings, save path and unsaved edits.
    let (bin, sequences) = s.edit("Import Project", |target, _| merge(target, fragment.clone(), parent, &name))?;
    s.media.add_stored(&fragment, Some(path));
    s.toast(format!("Imported project: {name} ({} sequences, {} missing media)", sequences.len(), missing.len()));
    Ok(
        json!({"bin":bin.0,"sequences":sequences.iter().map(|id| id.0).collect::<Vec<_>>(),"primarySequence":sequences.get(primary_index).map(|id|id.0),"missingMedia":missing}),
    )
}

// IDs share one monotonic namespace. Shift *references as well as definitions*, including
// dangling references, so a broken source reference can never bind to a destination object.
fn shifted(id: u64, offset: u64) -> Result<u64> {
    id.checked_add(offset).filter(|n| *n < Project::MAX_NEXT_ID).ok_or_else(|| error("Project ID space exhausted during import"))
}
fn mixer(m: &mut MixerStrip, offset: u64) -> Result<()> {
    for send in &mut m.sends {
        send.target.0 = shifted(send.target.0, offset)?;
    }
    if let Some(id) = &mut m.output {
        id.0 = shifted(id.0, offset)?;
    }
    Ok(())
}
fn effects(fx: &mut [EffectInstance], luts: &BTreeMap<String, String>) {
    for e in fx {
        // Only LUT parameters are references; never rewrite text/title contents.
        if e.effect != "lumetri" {
            continue;
        }
        for (name, p) in &mut e.params {
            if !matches!(name.as_str(), "input_lut" | "look_lut") {
                continue;
            }
            for v in std::iter::once(&mut p.value).chain(p.keyframes.iter_mut().map(|k| &mut k.value)) {
                if let ParamValue::Text(text) = v
                    && text.starts_with("lib:")
                {
                    // A missing source LUT must not accidentally bind to a destination LUT.
                    *text = luts.get(text).cloned().unwrap_or_default();
                }
            }
        }
    }
}
fn bins(bin: &mut Bin, offset: u64, depth: usize) -> Result<()> {
    if depth > 64 {
        return Err(error("Project folders are nested too deeply"));
    }
    bin.id.0 = shifted(bin.id.0, offset)?;
    for child in &mut bin.children {
        match child {
            BinEntry::Item(id) => id.0 = shifted(id.0, offset)?,
            BinEntry::Bin(b) => bins(b, offset, depth + 1)?,
        }
    }
    Ok(())
}
fn merge(target: &mut Project, mut source: Project, parent: Option<BinId>, name: &str) -> Result<(BinId, Vec<ItemId>)> {
    if parent.is_some_and(|p| target.root.find_bin(p).is_none()) {
        return Err(error("Import destination folder no longer exists"));
    }
    let offset = target.max_id().checked_add(1).ok_or_else(|| error("Project ID space exhausted"))?.max(target.next_id);
    let past = shifted(source.max_id(), offset)?.checked_add(1).ok_or_else(|| error("Project ID space exhausted"))?;
    let end = past.checked_add(source.luts.len() as u64).filter(|n| *n < Project::MAX_NEXT_ID).ok_or_else(|| error("Project ID space exhausted"))?;
    let mut luts = BTreeMap::new();
    for (index, lut) in source.luts.iter_mut().enumerate() {
        let new = format!("lut{}", past + index as u64);
        luts.insert(format!("lib:{}", lut.id), format!("lib:{new}"));
        lut.id = new;
    }
    bins(&mut source.root, offset, 0)?;
    source.root.name = name.to_string();
    let bin = source.root.id;
    let mut items = BTreeMap::new();
    let mut sequences = Vec::new();
    for (id, mut item) in source.items {
        if id != item.id {
            return Err(error("Project item ID does not match its map key"));
        }
        item.id.0 = shifted(id.0, offset)?;
        item.created = item.id.0;
        match &mut item.kind {
            ItemKind::Media(m) => {
                for mk in &mut m.markers {
                    mk.id.0 = shifted(mk.id.0, offset)?;
                }
            }
            ItemKind::Subclip { parent, .. } => parent.0 = shifted(parent.0, offset)?,
            ItemKind::Sequence(seq) => {
                sequences.push(item.id);
                let seq = Arc::make_mut(seq);
                mixer(&mut seq.master_mixer, offset)?;
                effects(&mut seq.master_effects, &luts);
                for mk in &mut seq.markers {
                    mk.id.0 = shifted(mk.id.0, offset)?;
                }
                for t in seq.video_tracks.iter_mut().chain(&mut seq.audio_tracks).chain(&mut seq.submix_tracks) {
                    t.id.0 = shifted(t.id.0, offset)?;
                    mixer(&mut t.mixer, offset)?;
                    effects(&mut t.effects, &luts);
                    for c in &mut t.items {
                        c.id.0 = shifted(c.id.0, offset)?;
                        c.item.0 = shifted(c.item.0, offset)?;
                        c.link = c.link.map(|id| shifted(id, offset)).transpose()?;
                        c.group = c.group.map(|id| shifted(id, offset)).transpose()?;
                        for mk in &mut c.markers {
                            mk.id.0 = shifted(mk.id.0, offset)?;
                        }
                        effects(&mut c.effects, &luts);
                    }
                    for tr in &mut t.transitions {
                        tr.id.0 = shifted(tr.id.0, offset)?;
                        if let Some(id) = &mut tr.from {
                            id.0 = shifted(id.0, offset)?;
                        }
                        if let Some(id) = &mut tr.to {
                            id.0 = shifted(id.0, offset)?;
                        }
                    }
                }
                for t in &mut seq.caption_tracks {
                    t.id.0 = shifted(t.id.0, offset)?;
                    for c in &mut t.captions {
                        c.id.0 = shifted(c.id.0, offset)?;
                    }
                }
                if let Some(mc) = &mut seq.multicam {
                    for cam in &mut mc.cameras {
                        for id in cam.video_track.iter_mut().chain(&mut cam.audio_tracks) {
                            id.0 = shifted(id.0, offset)?;
                        }
                        if let Some(id) = &mut cam.source {
                            id.0 = shifted(id.0, offset)?;
                        }
                    }
                }
                if let Some(m) = &mut seq.merged {
                    for id in m.video.iter_mut().chain(&mut m.audio) {
                        id.0 = shifted(id.0, offset)?;
                    }
                }
            }
            _ => {}
        }
        items.insert(item.id, item);
    }
    let mut transcripts = BTreeMap::new();
    for (id, v) in source.transcripts {
        transcripts.insert(ItemId(shifted(id.0, offset)?), v);
    }
    let mut graphics = BTreeMap::new();
    for (id, mut v) in source.source_graphics {
        effects(&mut v.layers, &luts);
        graphics.insert(ItemId(shifted(id.0, offset)?), v);
    }
    let mut narrations = BTreeMap::new();
    for (id, v) in source.narrations {
        narrations.insert(ItemId(shifted(id.0, offset)?), v);
    }
    for b in &mut source.search_bins {
        b.id.0 = shifted(b.id.0, offset)?;
    }
    // All fallible preparation is complete before changing the destination.
    target.next_id = end;
    target.items.extend(items);
    target.luts.extend(source.luts);
    target.transcripts.extend(transcripts);
    target.source_graphics.extend(graphics);
    target.narrations.extend(narrations);
    target.search_bins.extend(source.search_bins);
    if !source.notes.is_empty() {
        target.notes.push_str(&format!("\n\n{name}\n{}", source.notes));
    }
    let dest = match parent.and_then(|p| target.root.find_bin_mut(p)) {
        Some(b) => b,
        None => &mut target.root,
    };
    dest.children.push(BinEntry::Bin(source.root));
    Ok((bin, sequences))
}

#[cfg(test)]
mod tests;
