//! Bounded voice grammar for docked media workflows. Ordinals refer to visible result order.
use super::grammar::{Intent, number};
use serde_json::json;
pub(super) fn parse(s: &str) -> Option<Intent> {
    for (phrases, param, value) in [
        (&["включи хит режим", "shaker hit mode on"][..], "hit_mode", json!(true)),
        (&["выключи хит режим", "непрерывная тряска", "shaker hit mode off"][..], "hit_mode", json!(false)),
        (&["пресет шейкера ударный", "shaker preset hit"][..], "preset", json!(0)),
        (&["пресет шейкера эдит", "пресет шейкера edit", "shaker preset edit", "пресет шейкера эдт", "shaker preset edt"][..], "preset", json!(1)),
        (&["пресет шейкера с рук", "shaker preset handheld"][..], "preset", json!(2)),
    ] {
        if phrases.contains(&s) {
            return Some(Intent { id: "voice.parameter", params: json!({"effect":"shaker","param":param,"value":value}) });
        }
    }
    if let Some(rest) = s.strip_prefix("скачай качество ").or_else(|| s.strip_prefix("download quality ")) {
        let n = number(rest).filter(|n| [144, 240, 360, 480, 720, 1080, 1440, 2160, 4320].contains(n))?;
        return Some(Intent { id: "downloader.download", params: json!({"quality":n.to_string()}) });
    }
    if matches!(s, "скачай только аудио" | "download audio only") {
        return Some(Intent { id: "downloader.download", params: json!({"quality":"audio"}) });
    }
    for (prefix, id) in [
        ("оставь дубль ", "jarvis.keepTake"),
        ("keep take ", "jarvis.keepTake"),
        ("прослушай дубль ", "jarvis.previewTake"),
        ("preview take ", "jarvis.previewTake"),
    ] {
        if let Some(rest) = s.strip_prefix(prefix) {
            let (take, group) = rest.split_once(" в группе ").or_else(|| rest.split_once(" in group "))?;
            let take = ordinal(take).filter(|n| (1..=100).contains(n))?;
            let group = ordinal(group).filter(|n| (1..=100).contains(n))?;
            return Some(Intent { id, params: json!({"group":group-1,"keep":take-1,"take":take-1}) });
        }
    }
    if let Some(rest) = s.strip_prefix("выбери секвенцию для шортсов ").or_else(|| s.strip_prefix("shorts source sequence ")) {
        let n = ordinal(rest).filter(|n| (1..=100).contains(n))?;
        return Some(Intent { id: "jarvis.shortsSource", params: json!({"index":n}) });
    }
    for (prefix, id) in [
        ("найди звук ", "soundpad.search"),
        ("найти звук ", "soundpad.search"),
        ("поищи звуки ", "soundpad.search"),
        ("найди саунд ", "soundpad.search"),
        ("найди саунд эффект ", "soundpad.search"),
        ("найди звуки ", "soundpad.search"),
        ("поищи звук ", "soundpad.search"),
        ("поиск звука ", "soundpad.search"),
        ("найди в саундпаде ", "soundpad.search"),
        ("find sound ", "soundpad.search"),
        ("search sounds ", "soundpad.search"),
        ("search soundpad for ", "soundpad.search"),
        ("найди гифку ", "inserts.search"),
        ("поищи гифку ", "inserts.search"),
        ("найди вставку ", "inserts.search"),
        ("find gif ", "inserts.search"),
        ("search gifs ", "inserts.search"),
    ] {
        if let Some(q) = s.strip_prefix(prefix).filter(|q| !q.is_empty() && q.chars().count() <= 50) {
            return Some(Intent { id, params: json!({"query":q}) });
        }
    }
    for (prefix, id) in [
        ("послушай звук ", "soundpad.preview"),
        ("прослушай звук ", "soundpad.preview"),
        ("включи звук номер ", "soundpad.preview"),
        ("проиграй звук ", "soundpad.preview"),
        ("play sound ", "soundpad.preview"),
        ("preview sound ", "soundpad.preview"),
        ("добавь звук ", "soundpad.import"),
        ("импортируй звук ", "soundpad.import"),
        ("вставь звук ", "soundpad.import"),
        ("add sound ", "soundpad.import"),
        ("import sound ", "soundpad.import"),
        ("добавь гифку ", "jarvis.importGif"),
        ("import gif ", "jarvis.importGif"),
        ("выбери шортс ", "jarvis.shortsSelect"),
        ("выдели фрагмент ", "jarvis.shortsSelect"),
        ("select short ", "jarvis.shortsSelect"),
        ("покажи шортс ", "jarvis.shortsPreview"),
        ("просмотри фрагмент ", "jarvis.shortsPreview"),
        ("preview short ", "jarvis.shortsPreview"),
    ] {
        if let Some(rest) = s.strip_prefix(prefix) {
            let (rest, timeline) = if let Some(r) = rest.strip_suffix(" в проект").or_else(|| rest.strip_suffix(" into project")) {
                (r, Some(false))
            } else if let Some(r) = rest.strip_suffix(" на таймлайн").or_else(|| rest.strip_suffix(" to timeline")) {
                (r, Some(true))
            } else {
                (rest, None)
            };
            let rest = rest.strip_prefix("номер ").or_else(|| rest.strip_prefix("number ")).unwrap_or(rest);
            let n = ordinal(rest).filter(|n| (1..=50).contains(n))?;
            let mut p = json!({"index":n});
            if let Some(v) = timeline {
                p["timeline"] = json!(v);
            }
            return Some(Intent { id, params: p });
        }
    }
    for prefix in ["проанализируй шортсы на ", "найди фрагменты на ", "анализ шортсов на ", "analyze shorts for "]
    {
        if let Some(rest) = s.strip_prefix(prefix) {
            let n = number(rest.strip_suffix(" секунд").or_else(|| rest.strip_suffix(" seconds")).unwrap_or(rest)).filter(|n| [30, 60, 90].contains(n))?;
            return Some(Intent { id: "jarvis.shortsAnalyze", params: json!({"duration":n}) });
        }
    }
    if matches!(s, "выключи субтитры шортсов" | "shorts captions off") {
        return Some(Intent { id: "jarvis.shortsOptions", params: json!({"captions":false}) });
    }
    if matches!(s, "включи субтитры шортсов" | "shorts captions on") {
        return Some(Intent { id: "jarvis.shortsOptions", params: json!({"captions":true}) });
    }
    for prefix in ["сделай версии шортсов ", "вариации шортсов ", "shorts versions "] {
        if let Some(rest) = s.strip_prefix(prefix) {
            let n = number(rest).filter(|n| (1..=5).contains(n))?;
            return Some(Intent { id: "jarvis.shortsOptions", params: json!({"versions":n}) });
        }
    }
    for (phrases, key, value) in [
        (&["субтитры шортсов анимация поп", "shorts captions pop"][..], "animation", "pop"),
        (&["субтитры шортсов плавное появление", "shorts captions fade"][..], "animation", "fade"),
        (&["субтитры шортсов без анимации", "shorts captions no animation"][..], "animation", "none"),
        (&["субтитры шортсов минимальные", "shorts captions minimal"][..], "captionPreset", "minimal"),
        (&["субтитры шортсов с подложкой", "shorts captions boxed"][..], "captionPreset", "boxed"),
        (&["субтитры шортсов акцент", "shorts captions accent"][..], "captionPreset", "accent"),
        (&["шортсы с размытым фоном", "shorts blurred background"][..], "layout", "background"),
        (&["шортсы сплит скрин", "шортсы разделенный экран", "shorts split screen"][..], "layout", "split"),
        (&["шортсы автокадрирование", "shorts auto framing"][..], "layout", "auto"),
        (&["шортсы заполнить кадр", "shorts fill frame"][..], "layout", "fill"),
        (&["субтитры шортсов сверху", "shorts captions top"][..], "position", "top"),
        (&["субтитры шортсов снизу", "shorts captions bottom"][..], "position", "bottom"),
        (&["субтитры шортсов по центру", "shorts captions center"][..], "position", "center"),
        (&["вертикальный формат шортсов", "shorts portrait format"][..], "format", "9:16"),
        (&["квадратный формат шортсов", "shorts square format"][..], "format", "1:1"),
    ] {
        if phrases.contains(&s) {
            return Some(Intent { id: "jarvis.shortsOptions", params: json!({key:value}) });
        }
    }
    None
}
fn ordinal(s: &str) -> Option<i64> {
    match s {
        "первый" | "первую" | "первое" | "first" => Some(1),
        "второй" | "вторую" | "второе" | "second" => Some(2),
        "третий" | "третью" | "третье" | "third" => Some(3),
        "четвертый" | "четвертую" | "fourth" => Some(4),
        "пятый" | "пятую" | "fifth" => Some(5),
        _ => number(s),
    }
}
pub(super) fn execute(app: &mut crate::FilmcraftApp, id: &str, p: &serde_json::Value) -> Result<serde_json::Value, String> {
    use serde_json::Value;
    match id {
        "jarvis.keepTake" => app.session.execute("audioCleanup.apply", p.clone()).map_err(|e| e.to_string()),
        "jarvis.previewTake" => crate::panels::audio_cleanup::preview_command(app, p),
        "jarvis.soundNext" | "jarvis.soundPrevious" => {
            let state = app.soundpad.inspect();
            let page = state["page"].as_u64().unwrap_or(1);
            let page = if id == "jarvis.soundNext" {
                if state["next"] != true {
                    return Err("No next page".into());
                }
                page.saturating_add(1)
            } else {
                page.saturating_sub(1).max(1)
            };
            crate::soundpad::command(app, "soundpad.search", json!({"query":state["query"],"page":page}))
        }
        "jarvis.shortsSource" => {
            let i = index(p)?;
            let seq = app.session.project.sequences().nth(i).ok_or("Sequence number out of range")?.id;
            app.shorts.source = Some(seq);
            app.shorts.selected.clear();
            Ok(Value::Null)
        }
        "jarvis.shortsFolder" => {
            app.pick_ui(crate::native_dialogs::FileDialog::Folder { at: None }, |app, paths| {
                if let Some(p) = paths.into_iter().next() {
                    app.shorts.folder = p;
                }
            });
            Ok(Value::Null)
        }
        "jarvis.importGif" => {
            let i = index(p)?;
            let state = app.inserts.inspect();
            let id = state["results"].as_array().and_then(|r| r.get(i)).and_then(|r| r["id"].as_str()).ok_or("Choose a GIF from search results")?;
            crate::inserts::command(app, "inserts.import", json!({"id":id}))
        }
        "jarvis.shortsAnalyze" => {
            let duration = p["duration"].as_u64().unwrap_or(if app.shorts.duration == 0 { 60 } else { app.shorts.duration as u64 });
            let seq = app.shorts.source.or(app.session.state.active_sequence).ok_or("Select a sequence first")?;
            let r = app.session.execute("shorts.analyze", json!({"sequence":seq,"duration":duration})).map_err(|e| e.to_string())?;
            app.shorts.source = Some(seq);
            app.shorts.duration = duration as u32;
            app.shorts.selected.clear();
            app.show_panel(crate::dock::PanelKind::VerticalVideos);
            Ok(r)
        }
        "jarvis.shortsSelect" | "jarvis.shortsSelectAll" | "jarvis.shortsPreview" => {
            let status = app.session.execute("shorts.status", json!({})).map_err(|e| e.to_string())?;
            if status["valid"] != true {
                return Err("Analyze the source sequence first".into());
            }
            let rows = status["proposals"].as_array().ok_or("No proposals")?;
            if id == "jarvis.shortsSelectAll" {
                app.shorts.selected = (0..rows.len()).collect();
                return Ok(Value::Null);
            }
            let i = index(p)?;
            let row = rows.get(i).ok_or("Short number out of range")?;
            if id == "jarvis.shortsSelect" {
                app.shorts.selected.insert(i);
            } else {
                let start = row["startMs"].as_i64().ok_or("Invalid preview start")?;
                let end = row["endMs"].as_i64().ok_or("Invalid preview end")?;
                let end = end.checked_mul(filmcraft_time::TICKS_PER_SECOND / 1000).ok_or("Preview time overflow")?;
                app.stop();
                app.session.execute("sequence.open", json!({"item":status["source"]})).map_err(|e| e.to_string())?;
                app.session
                    .execute("playhead.set", json!({"time":start.checked_mul(filmcraft_time::TICKS_PER_SECOND/1000).ok_or("Preview time overflow")?}))
                    .map_err(|e| e.to_string())?;
                app.shorts.preview_source = app.session.state.active_sequence;
                app.shorts.preview_end = Some(filmcraft_time::Tick(end));
                app.play(1.);
            }
            Ok(Value::Null)
        }
        "jarvis.shortsCreate" => {
            let v = app.session.execute("shorts.create", json!({"selected":app.shorts.selected,"options":app.shorts.options})).map_err(|e| e.to_string())?;
            for id in v["sequences"].as_array().into_iter().flatten().filter_map(Value::as_u64).map(filmcraft_project::ItemId) {
                app.shorts.created.push(id);
                app.shorts.export_selected.insert(id);
            }
            Ok(v)
        }
        "jarvis.shortsExport" => {
            app.session.execute("shorts.export", json!({"sequences":app.shorts.export_selected,"folder":app.shorts.folder})).map_err(|e| e.to_string())
        }
        "jarvis.shortsOptions" => {
            let mut v = serde_json::to_value(&app.shorts.options).map_err(|e| e.to_string())?;
            for (k, x) in p.as_object().ok_or("Invalid options")? {
                v[k] = x.clone();
            }
            app.shorts.options = serde_json::from_value(v).map_err(|e| e.to_string())?;
            Ok(Value::Null)
        }
        "jarvis.cleanupSilence" | "jarvis.cleanupDuplicates" => {
            let duplicates = id == "jarvis.cleanupDuplicates";
            crate::panels::audio_cleanup::open(app, duplicates)?;
            app.session.execute("audioCleanup.analyze", json!({"duplicates":duplicates,"autoThreshold":true})).map_err(|e| e.to_string())
        }
        _ => Err("Unknown workflow voice command".into()),
    }
}
fn index(p: &serde_json::Value) -> Result<usize, String> {
    p["index"].as_u64().and_then(|n| usize::try_from(n).ok()).and_then(|n| n.checked_sub(1)).ok_or_else(|| "Result number required".into())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn workflow_variants_are_bounded() {
        for (s, id) in [
            ("найди звук удар", "soundpad.search"),
            ("play sound second", "soundpad.preview"),
            ("добавь звук три на таймлайн", "soundpad.import"),
            ("выбери шортс второй", "jarvis.shortsSelect"),
            ("проанализируй шортсы на шестьдесят секунд", "jarvis.shortsAnalyze"),
            ("найди гифку кот", "inserts.search"),
        ] {
            assert_eq!(parse(s).unwrap().id, id, "{s}");
        }
        for s in [
            "добавь звук 0",
            "добавь звук 9999999999999999999999",
            "play sound second and delete everything",
            "проанализируй шортсы на 999 секунд",
            "сделай версии шортсов 0",
        ] {
            assert!(parse(s).is_none(), "{s}");
        }
    }
}
