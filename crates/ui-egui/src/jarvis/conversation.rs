//! Conservative conversational rewriting, followed by the existing typed command grammar.
//! Never discard an unknown clause or execute a recognized prefix of a longer request.
use super::{Intent, json, natural, normalize, parse_single};

fn trim_intro(mut s: &str) -> &str {
    for _ in 0..4 {
        let next = [
            "я хочу чтобы ты ",
            "я хочу ",
            "хочу ",
            "можешь пожалуйста ",
            "можешь ",
            "давай ",
            "пожалуйста ",
            "could you please ",
            "could you ",
            "can you ",
            "i want you to ",
            "please ",
        ]
        .iter()
        .find_map(|p| s.strip_prefix(p));
        match next {
            Some(next) => s = next,
            None => break,
        }
    }
    s.trim()
}
fn clause(text: &str) -> Option<Intent> {
    if let Some(i) = parse_single(text) {
        return Some(i);
    }
    let n = normalize(text);
    let s = trim_intro(&n);
    // Keep named clips opaque: e.g. a title containing “then” is not a command separator.
    if s.contains("по имени ") || s.contains("named ") {
        return parse_single(s);
    }
    let mut body = s.to_string();
    let mut target = None;
    // Extract a target before the verb, or following a complete action.
    if let Some(rest) = s.strip_prefix("на ").or_else(|| s.strip_prefix("on ")) {
        for verb in [
            " сделать ",
            " уменьшить ",
            " увеличить ",
            " добавить ",
            " разрезать ",
            " сделай ",
            " уменьши ",
            " увеличь ",
            " понизь ",
            " повысь ",
            " добавь ",
            " разрежь ",
            " reduce ",
            " decrease ",
            " increase ",
            " add ",
            " split ",
            " set ",
        ] {
            if let Some((t, action)) = rest.split_once(verb) {
                target = Some(natural::target(t)?);
                body = format!("{} {}", verb.trim(), action);
                break;
            }
        }
    } else {
        for sep in [" на ", " on "] {
            if let Some((action, t)) = s.rsplit_once(sep)
                && (t.contains("клип") || t.contains("clip"))
            {
                target = Some(natural::target(t)?);
                body = action.into();
                break;
            }
        }
    }
    let mut words: Vec<&str> = body.split_whitespace().collect();
    // Pronouns refer to the current selection (including a preceding select step).
    words.retain(|w| !matches!(*w, "его" | "ее" | "ему" | "it"));
    if let Some(first) = words.first_mut() {
        let verbs = ["уменьши", "увеличь", "понизь", "повысь", "выдели", "разрежь", "добавь", "decrease", "increase", "select"];
        if !verbs.contains(first) {
            let matches: Vec<_> = verbs.into_iter().filter(|v| super::near_phrase(first, v)).collect();
            if matches.len() == 1 {
                *first = matches[0];
            }
        }
    }
    let mut body = words.join(" ");
    for (a, b) in [
        ("сделать ", "сделай "),
        ("уменьшить ", "уменьши "),
        ("увеличить ", "увеличь "),
        ("выделить ", "выдели "),
        ("добавить ", "добавь "),
        ("понизить ", "понизь "),
        ("reduce ", "decrease "),
        ("lower ", "decrease "),
        ("raise ", "increase "),
    ] {
        if let Some(rest) = body.strip_prefix(a) {
            body = format!("{b}{rest}");
            break;
        }
    }
    for (property, alias) in [("яркость", "яркость"), ("масштаб", "масштаб"), ("размер", "размер"), ("непрозрачность", "непрозрачность")]
    {
        for (word, verb) in [
            ("поменьше", "уменьши"),
            ("меньше", "уменьши"),
            ("пониже", "уменьши"),
            ("ниже", "уменьши"),
            ("побольше", "увеличь"),
            ("больше", "увеличь"),
            ("повыше", "увеличь"),
            ("выше", "увеличь"),
        ] {
            let form = format!("сделай {property} {word}");
            if let Some(tail) = body.strip_prefix(&form) {
                if tail.is_empty() {
                    return None;
                } // No silent “a little” numeric defaults.
                body = format!("{verb} {alias}{tail}");
                break;
            }
        }
    }
    let half = matches!(
        body.as_str(),
        "разрежь пополам"
            | "разрежь клип пополам"
            | "раздели пополам"
            | "раздели клип пополам"
            | "обрежь пополам"
            | "обрезать пополам"
            | "разрезать пополам"
            | "разделить пополам"
            | "split in half"
            | "split clip in half"
            | "split the clip in half"
            | "cut in half"
    );
    let mut intent = if half { Intent::new("voice.splitHalf", json!({})) } else { parse_single(&body)? };
    if let Some(t) = target {
        if !matches!(intent.id, "voice.parameter" | "voice.effect" | "voice.shake" | "voice.splitHalf" | "voice.duration" | "voice.reorder" | "jarvis.askShake")
        {
            return None;
        }
        intent.params["target"] = t;
    }
    Some(intent)
}
pub(super) fn parse(text: &str) -> Option<Intent> {
    if text.chars().count() > 512 {
        return None;
    }
    let n = normalize(text);
    if n.split_whitespace().any(|w| matches!(w, "не" | "нет" | "not" | "dont" | "don't" | "never" | "нельзя")) {
        return None;
    }
    // Preserve punctuation/numeric rejection in the single-command parser.
    if text.chars().zip(text.chars().skip(1)).any(|(a, b)| matches!(a, '-' | '−') && b.is_ascii_digit()) {
        return None;
    }
    if n.contains("по имени ") || n.contains("named ") {
        return clause(text);
    }
    let mut split = n.clone();
    for sep in
        [" после чего ", " после этого ", " и затем ", " а затем ", " и потом ", " а потом ", " затем ", " потом ", " and then ", " after that ", " then "]
    {
        split = split.replace(sep, " | ");
    }
    let parts: Vec<_> = split.split(" | ").collect();
    if parts.len() == 1 {
        return clause(text);
    }
    if parts.len() > 16 {
        return None;
    }
    let mut steps = Vec::new();
    for part in parts {
        let mut intent = clause(part.trim())?;
        if intent.id == "edit.rippleDelete" {
            intent.id = "voice.rippleDelete";
        }
        if !filmcraft_engine::voice_edit::plan_command_allowed(intent.id) {
            return None;
        }
        if intent.id != "voice.selectClip"
            && let Some(target) = intent.params.get("target").filter(|t| t.as_object().is_some_and(|o| !o.is_empty()))
        {
            steps.push(json!({"id":"voice.selectClip","params":{"target":target}}));
        }
        steps.push(json!({"id":intent.id,"params":intent.params}));
    }
    if steps.len() > 16 {
        return None;
    }
    Some(Intent::new("voice.plan", json!({"steps":steps})))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn jarvis_conversation_variants_and_chains() {
        for text in [
            "хочу на втором клипе сделать яркость поменьше на 20 процентов",
            "на втором клипе уменьши яркость на 20 процентов",
            "понизь яркость на 20 процентов на втором клипе",
        ] {
            let i = parse(text).unwrap();
            assert_eq!(i.id, "voice.parameter", "{text}");
            assert_eq!(i.params["target"]["ordinal"], 2);
            assert_eq!(i.params["value"], -20.0);
        }
        for text in [
            "выдели второй клип потом уменьши его масштаб на 20 процентов после чего разрежь его пополам",
            "select second clip then reduce scale by 20 percent and then split it in half",
        ] {
            let i = parse(text).unwrap();
            assert_eq!(i.id, "voice.plan");
            let steps = i.params["steps"].as_array().unwrap();
            assert_eq!(steps.len(), 3);
            assert_eq!(steps[2]["id"], "voice.splitHalf");
        }
        for text in ["уменьши масштаб на двадцать пять процентов", "reduce scale by twenty five percent", "уменши масштаб на 25 процентов"]
        {
            assert_eq!(parse(text).unwrap().params["value"], -25.0, "{text}");
        }
        let i = parse("на втором клипе уменьши яркость на 20 процентов потом разрежь его пополам").unwrap();
        assert_eq!(i.params["steps"][0]["id"], "voice.selectClip");
        assert_eq!(i.params["steps"][0]["params"]["target"]["ordinal"], 2);
        assert_eq!(parse("could you please lower brightness by 20 percent").unwrap().id, "voice.parameter");
    }
    #[test]
    fn jarvis_conversation_never_drops_unknown_or_negative_clauses() {
        for text in [
            "уменьши яркость на 20 процентов потом что то там сделай",
            "выдели второй клип потом не удаляй его",
            "выдели второй клип потом удали неизвестное",
            "хочу яркость поменьше",
            "выдели второй клип потом сделай эдит",
            "выдели второй клип потом отмени",
            "уменьши масштаб на -20 процентов",
            "выдели второй клип потом",
        ] {
            assert!(parse(text).is_none(), "{text}");
        }
    }
}
