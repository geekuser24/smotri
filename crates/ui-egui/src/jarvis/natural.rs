//! Bounded slot grammar: targets, effect names and numeric properties are never fuzzy guessed.
use super::{Intent, normalize};
use crate::i18n::Language;
use filmcraft_project::effect::{effect_defs, find_effect};
use serde_json::{Value, json};

fn ordinal(s: &str) -> Option<u64> {
    match s {
        "первого" | "первым" | "первое" | "первый" | "первом" | "первой" | "первую" | "first" => Some(1),
        "второго" | "вторым" | "второе" | "второй" | "втором" | "вторую" | "second" => Some(2),
        "третьего" | "третьим" | "третье" | "третий" | "третьем" | "третьей" | "третью" | "third" => Some(3),
        "четвертого" | "четвертым" | "четвертое" | "четвертый" | "четвертой" | "fourth" => Some(4),
        "пятого" | "пятым" | "пятое" | "пятый" | "пятой" | "fifth" => Some(5),
        "шестой" | "sixth" => Some(6),
        "седьмой" | "seventh" => Some(7),
        "восьмой" | "eighth" => Some(8),
        "девятый" | "ninth" => Some(9),
        "десятый" | "tenth" => Some(10),
        _ => super::number(s).and_then(|n| u64::try_from(n).ok()).filter(|n| *n > 0 && *n <= 10000),
    }
}
pub(super) fn target(s: &str) -> Option<Value> {
    let w: Vec<_> = s.split_whitespace().collect();
    let mut t = json!({});
    let mut i = 0;
    while let Some(word) = w.get(i).copied() {
        match word {
            "его" | "этот" | "выделенный" | "it" | "selected" | "на" | "on" | "the" | "клип" | "клипа" | "клипе" | "clip" | "номер" =>
                {}
            "видео" | "video" | "аудио" | "audio" | "дорожке" | "дорожка" | "track" => {
                if matches!(word, "аудио" | "audio") {
                    t["kind"] = json!("audio");
                }
                if let Some(n) = w.get(i + 1).and_then(|s| ordinal(s)) {
                    t["track"] = json!(n);
                    i += 1;
                }
            }
            "под" if w.get(i + 1) == Some(&"курсором") => {
                t["atPlayhead"] = json!(true);
                i += 1;
            }
            "at" if w.get(i + 1) == Some(&"playhead") => {
                t["atPlayhead"] = json!(true);
                i += 1;
            }
            "по" if w.get(i + 1) == Some(&"имени") => {
                let name = w.get(i + 2..)?.join(" ");
                if name.is_empty() {
                    return None;
                }
                t["name"] = json!(name);
                break;
            }
            "named" => {
                let name = w.get(i + 1..)?.join(" ");
                if name.is_empty() {
                    return None;
                }
                t["name"] = json!(name);
                break;
            }
            _ => {
                if let Some(n) = word.strip_prefix('v').and_then(ordinal) {
                    t["track"] = json!(n);
                    t["kind"] = json!("video");
                } else if let Some(n) = word.strip_prefix('a').and_then(ordinal) {
                    t["track"] = json!(n);
                    t["kind"] = json!("audio");
                } else {
                    let n = ordinal(word)?;
                    if w.get(i + 1).is_some_and(|s| matches!(*s, "дорожке" | "дорожка" | "track")) {
                        t["track"] = json!(n);
                        i += 2;
                        continue;
                    }
                    if t.get("ordinal").is_some() {
                        return None;
                    }
                    t["ordinal"] = json!(n);
                }
            }
        }
        i += 1;
    }
    Some(t)
}
fn effect_name(s: &str) -> Option<&'static str> {
    match s {
        "черно белый" | "черно белый эффект" | "чб" | "black and white" | "black white" | "обесцвечивание" => {
            return Some("black_white");
        }
        "яркость" | "brightness" => return Some("brightness_contrast"),
        _ => {}
    }
    let mut found = effect_defs().iter().filter(|d| [d.id, d.name, Language::Ru.tr(d.name)].iter().any(|n| normalize(n) == s));
    let first = found.next()?;
    found.next().is_none().then_some(first.id)
}
fn effect_action(s: &str) -> Option<Intent> {
    let mut placement = None;
    let mut s = s;
    for (suffix, value) in [
        (" корректирующим слоем", "layer"),
        (" на корректирующий слой", "layer"),
        (" поверх клипа", "layer"),
        (" поверх видео", "layer"),
        (" as adjustment layer", "layer"),
        (" на сам клип", "clip"),
        (" directly on clip", "clip"),
    ] {
        if let Some(rest) = s.strip_suffix(suffix) {
            s = rest;
            placement = Some(value);
            break;
        }
    }
    let mut phrase = s;
    let mut t = None;
    for sep in [" на ", " on "] {
        if let Some((a, b)) = s.split_once(sep)
            && (b.contains("клип") || b.contains("clip"))
        {
            phrase = a;
            t = Some(target(b)?);
            break;
        }
    }
    phrase = phrase.strip_suffix(" на него").or_else(|| phrase.strip_suffix(" на клип")).unwrap_or(phrase);
    if matches!(phrase, "шейк" | "тряску" | "shake" | "здесь тряску" | "здесь шейк" | "тряску здесь" | "shake here") {
        let mut p = json!({});
        if let Some(t) = t {
            p["target"] = t;
        }
        if let Some(placement) = placement {
            p["placement"] = json!(placement);
        }
        return Some(Intent::new("jarvis.askShake", p));
    }
    let preset = match phrase {
        "hit shake" | "ударный шейк" | "короткий шейк" | "короткий удар" | "short shake" => Some("hit"),
        "шейк как в edt" | "шейк edt" | "edt shake" | "шейк со вспышкой" => Some("edt"),
        "тряску камеры" | "живую тряску" | "тряску с рук" | "handheld shake" | "camera shake" => Some("handheld"),
        _ => None,
    };
    let mut p = if let Some(preset) = preset {
        json!({"preset":preset})
    } else {
        json!({"effect":effect_name(phrase.strip_prefix("эффект ").or_else(||phrase.strip_prefix("effect ")).unwrap_or(phrase))?})
    };
    if let Some(t) = t {
        p["target"] = t;
    }
    if let Some(placement) = placement {
        p["placement"] = json!(placement);
    }
    Some(Intent::new(if preset.is_some() { "voice.shake" } else { "voice.effect" }, p))
}
fn property(s: &str) -> Option<Intent> {
    let rest = s.strip_prefix("установи параметр ").or_else(|| s.strip_prefix("set parameter ")).or_else(|| s.strip_prefix("поставь параметр "))?;
    let (body, effect) = if let Some((b, e)) = rest.split_once(" в эффекте ").or_else(|| rest.split_once(" in effect ")) {
        (b, Some(effect_name(e)?))
    } else {
        (rest, None)
    };
    let (label, value) = body.split_once(" на ").or_else(|| body.split_once(" to ")).or_else(|| body.split_once(" значение "))?;
    let defs: Vec<_> = effect_defs()
        .iter()
        .filter(|d| effect.is_none_or(|id| d.id == id))
        .flat_map(|d| d.params.iter())
        .filter(|p| [p.id, p.label, Language::Ru.tr(p.label)].iter().any(|n| normalize(n) == label))
        .collect();
    let first = defs.first()?;
    if defs.iter().any(|p| p.id != first.id || p.kind != first.kind) {
        return None;
    }
    let val = match &first.kind {
        filmcraft_project::effect::ParamKind::Choice(options) => {
            json!(options.iter().find(|o| normalize(o) == value || normalize(Language::Ru.tr(o)) == value)?)
        }
        filmcraft_project::effect::ParamKind::Bool => json!(match value {
            "да" | "включено" | "on" | "true" => true,
            "выключено" | "off" | "false" => false,
            _ => return None,
        }),
        filmcraft_project::effect::ParamKind::Float { .. } | filmcraft_project::effect::ParamKind::Angle => {
            let (sign, v) = if let Some(v) = value.strip_prefix("минус ").or_else(|| value.strip_prefix("minus ")) { (-1., v) } else { (1., value) };
            let n = if let Some((a, b)) = v.split_once(" точка ").or_else(|| v.split_once(" point ")) {
                format!("{}.{}", super::number(a)?, super::number(b)?).parse::<f64>().ok()?
            } else {
                super::number(v)? as f64
            };
            json!(n * sign)
        }
        _ => return None,
    };
    let mut params = json!({"param":first.id,"value":val});
    if let Some(e) = effect {
        params["effect"] = json!(e);
    }
    Some(Intent::new("voice.parameter", params))
}
fn seconds_number(s: &str) -> Option<f64> {
    match s {
        "одну" | "одной" => Some(1.0),
        "двух" => Some(2.0),
        "трех" => Some(3.0),
        "полторы" => Some(1.5),
        _ => super::number(s).map(|n| n as f64),
    }
}
fn editing_action(s: &str) -> Option<Intent> {
    for verb in ["сделай", "поставь", "перемести"] {
        if let Some((source, dest)) = s.split_once(&format!(" клип {verb} "))
            && let Some(n) = ordinal(dest)
        {
            return Some(Intent::new("voice.reorder", json!({"target":target(source)?,"position":n})));
        }
    }

    for prefix in ["разрежь ", "разрезать ", "раздели ", "разделить ", "split ", "cut "] {
        if let Some(rest) = s.strip_prefix(prefix) {
            for suffix in ["пополам", "по центру", "посередине", "посредине", "in half", "at center"] {
                if let Some(t) = rest.strip_suffix(suffix) {
                    let t = t.trim().trim_matches(' ');
                    return Some(Intent::new("voice.splitHalf", json!({"target":target(t)?})));
                }
            }
        }
    }
    for prefix in ["сделай чтобы сначала шел ", "сначала поставь ", "put first "] {
        if let Some(t) = s.strip_prefix(prefix) {
            return Some(Intent::new("voice.reorder", json!({"target":target(t)?,"position":1})));
        }
    }
    for prefix in ["поставь ", "перемести ", "передвинь ", "сделай ", "move ", "put ", "make "] {
        if let Some(rest) = s.strip_prefix(prefix) {
            for sep in [" клип ", " clip "] {
                if let Some((source, dest)) = rest.split_once(sep) {
                    let dest = dest
                        .trim_start_matches("на ")
                        .trim_start_matches("в ")
                        .trim_start_matches("to ")
                        .trim_end_matches(" место")
                        .trim_end_matches(" position");
                    if let Some(n) = if matches!(dest, "начало" | "начале" | "самое начало" | "начало первого" | "start") {
                        Some(1)
                    } else {
                        ordinal(dest)
                    } {
                        return Some(Intent::new("voice.reorder", json!({"target":target(source)?,"position":n})));
                    }
                }
            }
        }
    }
    for prefix in ["сделай ", "оставь ", "обрежь ", "сократи ", "установи длительность ", "set duration ", "trim ", "make "]
    {
        if let Some(rest) = s.strip_prefix(prefix) {
            let words: Vec<_> = rest.split_whitespace().collect();
            let at = words.iter().rposition(|w| matches!(*w, "секунды" | "секунда" | "секунд" | "секунду" | "seconds" | "second"))?;
            for len in [2, 1] {
                if at < len {
                    continue;
                }
                let Some(n) = seconds_number(&words.get(at - len..at)?.join(" ")).filter(|n| *n > 0.0 && *n <= 86400.0) else { continue };
                let mut t = words.get(..at - len)?.to_vec();
                t.extend_from_slice(words.get(at + 1..)?);
                t.retain(|w| !matches!(*w, "до" | "на" | "длиной" | "длительностью" | "to" | "of" | "long"));
                if let Some(target) = target(&t.join(" ")) {
                    return Some(Intent::new("voice.duration", json!({"target":target,"seconds":n})));
                }
            }
        }
    }
    for prefix in ["добавь маркер ", "поставь маркер ", "создай маркер ", "add marker ", "set marker "] {
        if let Some(rest) = s.strip_prefix(prefix) {
            let rest = rest.trim_start_matches("с названием ").trim_start_matches("под названием ").trim_start_matches("named ");
            if let Some(time) = rest.strip_prefix("на ").or_else(|| rest.strip_prefix("at ")) {
                let mut words: Vec<_> = time.split_whitespace().collect();
                if matches!(words.last(), Some(&"секунд" | &"секунде" | &"секунды" | &"seconds" | &"second")) {
                    words.pop();
                }
                let seconds = seconds_number(&words.join(" "))?;
                if !(0.0..=86400.0).contains(&seconds) {
                    return None;
                }
                return Some(Intent::new("markers.add", json!({"target":"program","seconds":seconds})));
            }
            if !rest.is_empty() {
                return Some(Intent::new("markers.add", json!({"target":"program","name":rest})));
            }
        }
    }
    None
}
pub(super) fn parse(s: &str) -> Option<Intent> {
    if let Some(i) = editing_action(s) {
        return Some(i);
    }
    if ["создай ", "создать ", "добавь ", "добавить ", "сделай ", "create ", "add "].iter().any(|prefix| {
        s.strip_prefix(prefix).is_some_and(|rest| matches!(rest, "корректирующий слой" | "корректирующий слои" | "adjustment layer" | "an adjustment layer"))
    }) {
        return Some(Intent::new("voice.adjustmentLayer", json!({})));
    }

    if matches!(s, "выбери шейк" | "выбрать шейк" | "выбери тряску" | "choose shake" | "choose a shake") {
        return Some(Intent::new("jarvis.askShake", json!({})));
    }
    for prefix in ["выбери эффект ", "выдели эффект ", "select effect "] {
        if let Some(name) = s.strip_prefix(prefix) {
            return Some(Intent::new("voice.selectEffect", json!({"effect":effect_name(name)?})));
        }
    }
    if let Some(p) = property(s) {
        return Some(p);
    }
    for (prefix, remove) in
        [("расшифруй ", false), ("расшифровать ", false), ("transcribe ", false), ("удали паузы", true), ("убери паузы", true), ("remove pauses", true)]
    {
        if let Some(rest) = s.strip_prefix(prefix) {
            let mode = if rest.contains("голос") || rest.contains("voice") { "voice" } else { "all" };
            let clean = rest
                .replace("учитывая все слои", "")
                .replace("учитывай все слои", "")
                .replace("только учитывай голос", "")
                .replace("только голос", "")
                .replace("в голосе", "")
                .replace("voice only", "")
                .replace("all layers", "")
                .replace("голоса", "");
            let clean = clean.trim().trim_start_matches("в ").trim_start_matches("на ").trim_start_matches("in ").trim_start_matches("on ").trim();
            let mut params = json!({"mode":mode});
            if matches!(clean, "" | "таймлайн" | "таймлайне" | "везде" | "все" | "timeline" | "everywhere" | "all") {
                params["scope"] = json!("timeline");
            } else if matches!(clean, "клип" | "клипе" | "этот клип" | "выделенный клип" | "clip" | "selected clip") {
                params["scope"] = json!("clip");
            } else {
                params["scope"] = json!("clip");
                params["target"] = target(clean)?;
            }
            return Some(Intent::new(if remove { "voice.removePauses" } else { "voice.transcribe" }, params));
        }
    }

    for prefix in ["сдвинь таймлайн ", "перемести курсор ", "move playhead "] {
        if let Some(rest) = s.strip_prefix(prefix) {
            let w: Vec<_> = rest.split_whitespace().filter(|w| !matches!(*w, "на" | "by")).collect();
            if let [dir, n, "кадров" | "кадра" | "кадр" | "frames" | "frame"] = w.as_slice() {
                let sign = match *dir {
                    "вправо" | "вперед" | "right" | "forward" => 1,
                    "влево" | "назад" | "left" | "back" => -1,
                    _ => return None,
                };
                let n = super::number(n).filter(|n| (1..=100000).contains(n))?;
                return Some(Intent::new("playhead.step", json!({"frames":n*sign,"target":"program"})));
            }
        }
    }
    for prefix in ["выдели ", "выбери ", "выделить ", "select "] {
        if let Some(rest) = s.strip_prefix(prefix) {
            if !rest.contains("клип") && !rest.contains("clip") {
                return None;
            }
            let t = target(rest)?;
            return Some(Intent::new("voice.selectClip", json!({"target":if t.as_object()?.is_empty(){json!({"atPlayhead":true})}else{t}})));
        }
    }
    if matches!(s, "перейди к следующему клипу" | "go to next clip") {
        return Some(Intent::new("timeline.selectNextClip", json!({})));
    }
    if matches!(s, "перейди к предыдущему клипу" | "go to previous clip") {
        return Some(Intent::new("timeline.selectPrevClip", json!({})));
    }
    for prefix in ["добавь ", "накинь ", "наложи ", "сделай ", "add ", "apply "] {
        if let Some(rest) = s.strip_prefix(prefix)
            && let Some(i) = effect_action(rest)
        {
            return Some(i);
        }
    }
    for prefix in ["смени метод наложения на ", "режим наложения ", "set blend mode to ", "blend mode "] {
        if let Some(mode) = s.strip_prefix(prefix) {
            let d = find_effect("opacity")?;
            let p = d.params.iter().find(|p| p.id == "blend")?;
            if let filmcraft_project::effect::ParamKind::Choice(options) = &p.kind {
                let choice = options.iter().find(|x| normalize(x) == mode || normalize(Language::Ru.tr(x)) == mode)?;
                return Some(Intent::new("voice.parameter", json!({"effect":"opacity","param":"blend","value":choice,"ensure":true})));
            }
        }
    }
    for (prefix, op, sign) in [
        ("уменьши ", "add", -1.),
        ("понизь ", "add", -1.),
        ("увеличь ", "add", 1.),
        ("повысь ", "add", 1.),
        ("decrease ", "add", -1.),
        ("increase ", "add", 1.),
        ("сделай ниже ", "add", -1.),
        ("сделай выше ", "add", 1.),
        ("сделай меньше ", "add", -1.),
        ("сделай больше ", "add", 1.),
        ("сделай ", "set", 1.),
        ("установи ", "set", 1.),
        ("set ", "set", 1.),
    ] {
        if let Some(rest) = s.strip_prefix(prefix) {
            let mut w: Vec<_> = rest.split_whitespace().collect();
            // Unqualified percentage requests refer to clip scale.
            if w.first().is_some_and(|w| matches!(*w, "на" | "to" | "by") || super::number(w).is_some()) {
                w.insert(0, "масштаб");
            }
            let (first, remaining) = w.split_first()?;
            let (effect, param, _delta) = match *first {
                "scale" | "масштаб" | "размер" | "размеры" => ("motion", "scale", false),
                "яркость" | "brightness" => ("brightness_contrast", "brightness", true),
                "непрозрачность" | "opacity" => ("opacity", "opacity", true),
                _ => return None,
            };
            let values: Vec<_> = remaining
                .iter()
                .copied()
                .filter(|s| {
                    !matches!(
                        *s,
                        "на" | "в"
                            | "to"
                            | "by"
                            | "процентов"
                            | "процента"
                            | "процент"
                            | "percent"
                            | "пунктов"
                            | "пункта"
                            | "единиц"
                            | "значений"
                    )
                })
                .collect();
            if values.is_empty() || values.len() > 2 {
                return None;
            }
            let n = super::number(&values.join(" "))?;
            if !(0..=10000).contains(&n) {
                return None;
            }
            return Some(Intent::new("voice.parameter", json!({"effect":effect,"param":param,"value":n as f64*sign,"op":op,"ensure":true})));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    #[test]
    fn editing_duration_reorder_split_and_markers() {
        for (phrase, id) in [
            ("разрежь второй клип по центру", "voice.splitHalf"),
            ("раздели выделенный клип пополам", "voice.splitHalf"),
            ("сделай две секунды второго клипа", "voice.duration"),
            ("обрежь второй клип до двух секунд", "voice.duration"),
            ("make second clip two seconds", "voice.duration"),
            ("передвинь второй клип на первое место", "voice.reorder"),
            ("третий клип сделай вторым", "voice.reorder"),
            ("move third clip to second position", "voice.reorder"),
            ("поставь маркер на 10 секунд", "markers.add"),
            ("добавь маркер под названием удар", "markers.add"),
        ] {
            assert_eq!(super::parse(phrase).unwrap().id, id, "{phrase}");
        }
        assert_eq!(super::parse("сделай две секунды второго клипа").unwrap().params, serde_json::json!({"target":{"ordinal":2},"seconds":2.0}));
        let i = super::parse("сделай ниже на 20").unwrap();
        assert_eq!(i.params["value"], -20.0);
        assert_eq!(i.params["op"], "add");
        assert_eq!(super::parse("сделай 100 percent").unwrap().params["value"], 100.0);
        assert_eq!(super::parse("сделай 10 percent").unwrap().params["value"], 10.0);
    }

    #[test]
    fn adjustment_and_layer_shake_phrases() {
        for phrase in
            ["создать корректирующий слой", "создай корректирующий слой", "добавь корректирующий слой", "create adjustment layer", "add an adjustment layer"]
        {
            assert_eq!(super::parse(phrase).unwrap().id, "voice.adjustmentLayer", "{phrase}");
        }
        let i = super::parse("добавь короткий шейк корректирующим слоем").unwrap();
        assert_eq!(i.id, "voice.shake");
        assert_eq!(i.params["placement"], "layer");
        assert_eq!(super::parse("добавь ударный шейк на сам клип").unwrap().params["placement"], "clip");
    }

    use super::*;
    #[test]
    fn contextual_phrases() {
        for (phrase, id, p) in [
            ("выдели клип", "voice.selectClip", json!({"target":{"atPlayhead":true}})),
            ("выдели второй клип на видео 2", "voice.selectClip", json!({"target":{"ordinal":2,"track":2}})),
            ("select third audio clip on track 2", "voice.selectClip", json!({"target":{"ordinal":3,"track":2,"kind":"audio"}})),
            ("уменьши scale на 20 процентов", "voice.parameter", json!({"effect":"motion","param":"scale","value":-20.0,"op":"add","ensure":true})),
            (
                "понизь яркость на 20 процентов",
                "voice.parameter",
                json!({"effect":"brightness_contrast","param":"brightness","value":-20.0,"op":"add","ensure":true}),
            ),
            ("сдвинь таймлайн влево на 12 кадров", "playhead.step", json!({"frames":-12,"target":"program"})),
            ("расшифруй таймлайн", "voice.transcribe", json!({"scope":"timeline","mode":"all"})),
            ("удали паузы в голосе в клипе", "voice.removePauses", json!({"scope":"clip","mode":"voice"})),
            ("удали паузы учитывая все слои", "voice.removePauses", json!({"scope":"timeline","mode":"all"})),
            ("установи параметр frequency на 2 в эффекте camera shake", "voice.parameter", json!({"effect":"camera_shake","param":"frequency","value":2.0})),
            ("добавь шейк", "jarvis.askShake", json!({})),
            ("добавь тряску камеры", "voice.shake", json!({"preset":"handheld"})),
            ("сделай шейк как в edt", "voice.shake", json!({"preset":"edt"})),
            ("накинь черно белый эффект на второй клип на видео 2", "voice.effect", json!({"effect":"black_white","target":{"ordinal":2,"track":2}})),
        ] {
            assert_eq!(super::super::parse(phrase), Some(Intent::new(id, p)), "{phrase}");
        }
        assert_eq!(super::super::parse("отменить").unwrap().id, "edit.undo");
        assert!(super::super::parse("выдели сто тысяч клипов").is_none());
        assert!(super::super::parse("не добавь шейк").is_none());
    }
}
