use super::{near_phrase, normalize};
#[path = "conversation.rs"]
mod conversation;
#[path = "natural.rs"]
mod natural;
use crate::{i18n::Language, menus};
use serde_json::{Value, json};
use std::sync::OnceLock;

pub(super) const COMMANDS: &[(&str, &[&str])] = &[
    ("voice.shakerHit", &["поставь ключ удара", "добавь ключ шейкера", "удар шейкера здесь", "add shaker hit", "trigger shaker here"]),
    ("jarvis.soundNext", &["следующие звуки", "следующая страница звуков", "next sounds", "next sound page"]),
    ("jarvis.soundPrevious", &["предыдущие звуки", "previous sounds"]),
    ("jarvis.shortsFolder", &["выбери папку для шортсов", "choose shorts export folder"]),
    (
        "window.panel.soundPad",
        &["открой саундпад", "покажи саундпад", "саундпад", "открой звуковую библиотеку", "soundpad", "open soundpad", "show soundpad", "open sound library"],
    ),
    ("window.panel.verticalVideos", &["открой вертикальные видео", "покажи шортсы", "открой шортсы", "open vertical videos", "show shorts", "open shorts"]),
    ("window.panel.inserts", &["открой вставки", "покажи гифки", "open inserts", "show gifs"]),
    ("window.panel.downloader", &["открой загрузчик", "покажи загрузчик", "open downloader", "show downloader"]),
    ("window.panel.silenceRemoval", &["открой удаление тишины", "настрой удаление пауз", "open silence removal"]),
    ("window.panel.duplicateTakes", &["открой удаление дублей", "покажи дубли", "open duplicate takes"]),
    ("soundpad.stop", &["останови звук", "останови предпрослушивание", "stop sound", "stop sound preview"]),
    ("jarvis.shortsAnalyze", &["проанализируй шортсы", "найди короткие фрагменты", "анализ вертикальных видео", "analyze shorts", "find short clips"]),
    ("jarvis.shortsSelectAll", &["выбери все шортсы", "выдели все фрагменты", "select all shorts"]),
    ("jarvis.shortsCreate", &["создай выбранные шортсы", "создай шортсы", "create selected shorts", "create shorts"]),
    ("jarvis.shortsExport", &["экспортируй шортсы", "экспорт выбранных шортсов", "export shorts", "batch export shorts"]),
    ("jarvis.cleanupSilence", &["проанализируй тишину", "анализ пауз", "найди тишину в выделенном", "analyze silence", "find silent gaps"]),
    ("jarvis.cleanupDuplicates", &["проанализируй дубли", "найди повторные дубли", "analyze duplicate takes", "find repeated takes"]),
    ("audioCleanup.apply", &["примени удаление тишины", "apply silence removal"]),
    ("audioCleanup.cancel", &["отмени анализ звука", "cancel audio analysis"]),
    ("downloader.probe", &["проверь ссылку загрузчика", "проверить ссылку", "check download link"]),
    ("downloader.download", &["начни загрузку", "скачай видео по ссылке", "start download"]),
    ("downloader.cancel", &["отмени загрузку", "cancel download"]),
    ("downloader.import", &["импортируй скачанное", "import download"]),
    ("voice.autoEdit", &["edit", "эдит", "сделай edit", "сделай эдит", "сделай монтаж под музыку", "make an edit", "make edit", "create beat edit"]),
    (
        "sequence.addEdit",
        &[
            "нарезать",
            "нарежь",
            "разрежь",
            "разрезать",
            "раздели",
            "разделить",
            "сделай разрез",
            "split",
            "split at playhead",
            "add edit",
            "разрежь под курсором",
        ],
    ),
    ("edit.copy", &["копировать", "скопировать", "скопируй", "копируй", "copy"]),
    ("edit.cut", &["вырезать", "вырежи", "вырежь", "cut", "cut to clipboard"]),
    ("edit.paste", &["вставить", "вставь", "paste"]),
    ("edit.clear", &["удалить", "удали", "убери", "delete", "remove", "clear"]),
    (
        "edit.rippleDelete",
        &["удалить со сдвигом", "удали со сдвигом", "удали со смещением", "удалить со смещением", "удали без зазора", "ripple delete", "delete and close gap"],
    ),
    ("edit.undo", &["отмени", "отменить", "отмени действие", "назад", "undo", "undo last action"]),
    ("edit.redo", &["повтори", "верни", "верни действие", "redo"]),
    ("jarvis.play", &["воспроизведение", "воспроизведи", "играй", "play", "start playback"]),
    ("jarvis.pause", &["стоп", "пауза", "останови", "stop", "pause", "stop playback"]),
    ("jarvis.stop", &["выключись", "выключи микрофон", "выключи голосовой режим", "stop listening", "microphone off"]),
    ("edit.selectAll", &["выдели все", "выделить все", "select all"]),
    ("edit.deselectAll", &["сними выделение", "снять выделение", "deselect all", "deselect"]),
    ("edit.pasteInsert", &["вставь со сдвигом", "вставить со сдвигом", "paste insert"]),
    ("edit.duplicate", &["дублируй в проекте", "дублировать в проекте", "duplicate project item"]),
    ("sequence.addEditAllTracks", &["разрежь все дорожки", "разрезать все дорожки", "split all tracks"]),
    ("sequence.closeGap", &["закрой зазор", "убери зазор", "close gap"]),
    ("sequence.lift", &["удали диапазон", "lift range"]),
    ("sequence.extract", &["извлеки диапазон", "extract range"]),
    ("sequence.snap", &["переключи привязку", "toggle snapping"]),
    ("sequence.linkedSelection", &["переключи связанное выделение", "toggle linked selection"]),
    ("sequence.applyVideoTransition", &["добавь видеопереход", "добавь переход", "add video transition"]),
    ("sequence.applyAudioTransition", &["добавь аудиопереход", "add audio transition"]),
    ("sequence.removeTransition", &["удали переход", "remove transition"]),
    ("sequence.addTracks", &["добавить дорожки", "добавь дорожки", "add tracks"]),
    ("sequence.deleteTracks", &["удалить дорожки", "удали дорожки", "delete tracks"]),
    ("sequence.settings", &["настройки секвенции", "настройки эпизода", "sequence settings"]),
    ("sequence.renderInToOut", &["просчитай диапазон", "render in to out"]),
    ("sequence.renderSelection", &["просчитай выделенное", "render selection"]),
    ("sequence.matchFrame", &["найди исходный кадр", "match frame"]),
    ("clip.group", &["сгруппируй", "сгруппировать", "group"]),
    ("clip.ungroup", &["разгруппируй", "разгруппировать", "ungroup"]),
    ("clip.link", &["переключи связь клипов", "toggle clip link"]),
    ("clip.enable", &["переключи активность клипов", "toggle clip enabled"]),
    ("clip.nest", &["создай вложенную секвенцию", "nest"]),
    ("clip.rename", &["переименуй", "rename"]),
    ("clip.speedDuration", &["настрой скорость", "скорость и длительность", "speed duration"]),
    ("clip.audioGain", &["настрой усиление", "усиление звука", "audio gain"]),
    ("clip.volumeUp", &["громче", "увеличь громкость", "louder"]),
    ("clip.volumeDown", &["тише", "уменьши громкость", "quieter"]),
    ("clip.scaleToFrameSize", &["впиши в размер кадра", "scale to frame size"]),
    ("clip.frameHold", &["добавь стоп кадр", "стоп кадр", "freeze frame"]),
    ("clip.revealInProject", &["покажи в проекте", "reveal in project"]),
    ("playhead.start", &["в начало", "в начало таймлайна", "go to start"]),
    ("playhead.end", &["в конец", "в конец таймлайна", "go to end"]),
    ("playhead.stepForward", &["кадр вперед", "следующий кадр", "next frame"]),
    ("playhead.stepBack", &["кадр назад", "предыдущий кадр", "previous frame"]),
    ("playhead.nextEdit", &["следующая склейка", "следующий разрез", "next edit"]),
    ("playhead.prevEdit", &["предыдущая склейка", "предыдущий разрез", "previous edit"]),
    ("playhead.selectedClipStart", &["в начало клипа", "go to clip start"]),
    ("playhead.selectedClipEnd", &["в конец клипа", "go to clip end"]),
    ("timeline.selectClipAtPlayhead", &["выдели под курсором", "выдели клип под курсором", "select clip at playhead"]),
    ("timeline.selectNextClip", &["выдели следующий клип", "следующий клип", "select next clip"]),
    ("timeline.selectPrevClip", &["выдели предыдущий клип", "предыдущий клип", "select previous clip"]),
    ("timeline.nudgeLeft", &["сдвинь клип влево", "move clip left"]),
    ("timeline.nudgeRight", &["сдвинь клип вправо", "move clip right"]),
    ("timeline.nudgeUp", &["перемести клип на дорожку выше", "move clip up"]),
    ("timeline.nudgeDown", &["перемести клип на дорожку ниже", "move clip down"]),
    ("trim.extendPreviousEdit", &["продли предыдущую склейку до курсора", "extend previous edit to playhead"]),
    ("trim.extendNextEdit", &["продли следующую склейку до курсора", "extend next edit to playhead"]),
    ("markers.markIn", &["поставь вход", "точка входа", "mark in"]),
    ("markers.markOut", &["поставь выход", "точка выхода", "mark out"]),
    ("markers.markClip", &["отметь клип", "mark clip"]),
    ("markers.markSelection", &["отметь выделение", "mark selection"]),
    ("markers.clearIn", &["убери точку входа", "clear in"]),
    ("markers.clearOut", &["убери точку выхода", "clear out"]),
    ("markers.clearInOut", &["убери вход и выход", "clear in and out"]),
    ("markers.goToIn", &["к точке входа", "go to in"]),
    ("markers.goToOut", &["к точке выхода", "go to out"]),
    ("markers.add", &["поставь маркер", "добавь маркер", "add marker"]),
    ("markers.goNext", &["следующий маркер", "next marker"]),
    ("markers.goPrev", &["предыдущий маркер", "previous marker"]),
    ("markers.clearCurrent", &["удали маркер", "delete marker"]),
    ("markers.clearAll", &["удали все маркеры", "clear markers"]),
    ("playback.loop", &["зацикли воспроизведение", "переключи зацикливание", "toggle loop"]),
    ("playback.reverse", &["воспроизведи назад", "play backwards"]),
    ("playback.inToOut", &["воспроизведи диапазон", "play in to out"]),
    ("source.playback.play", &["воспроизведи источник", "play source"]),
    ("source.playback.stop", &["останови источник", "stop source"]),
    ("view.zoomIn", &["приблизь", "увеличь масштаб", "zoom in"]),
    ("view.zoomOut", &["отдали", "уменьши масштаб", "zoom out"]),
    ("view.zoomToSequence", &["покажи весь таймлайн", "вписать таймлайн", "fit timeline"]),
    ("tool.selection", &["инструмент выделения", "выбери стрелку", "selection tool"]),
    ("tool.razor", &["инструмент лезвие", "выбери лезвие", "razor tool"]),
    ("tool.ripple", &["инструмент монтажа со сдвигом", "ripple edit tool"]),
    ("tool.rolling", &["инструмент совмещенного монтажа", "rolling edit tool"]),
    ("tool.slip", &["инструмент смещения", "slip tool"]),
    ("tool.slide", &["инструмент прокрутки", "slide tool"]),
    ("tool.rateStretch", &["инструмент растяжения", "rate stretch tool"]),
    ("tool.hand", &["инструмент рука", "hand tool"]),
    ("tool.pen", &["инструмент перо", "pen tool"]),
    ("tool.type", &["инструмент текст", "type tool"]),
    ("tool.zoom", &["инструмент масштаб", "zoom tool"]),
    ("tool.trackSelectForward", &["инструмент выделения дорожки вперед", "track select forward tool"]),
    ("tool.trackSelectBackward", &["инструмент выделения дорожки назад", "track select backward tool"]),
    ("file.save", &["сохрани", "сохрани проект", "save project"]),
    ("file.saveAs", &["сохрани как", "save as"]),
    ("file.open", &["открой проект", "open project"]),
    ("file.import", &["импортируй", "импортировать", "импорт медиа", "import media"]),
    ("file.exportMedia", &["экспорт видео", "экспортируй видео", "export video"]),
    ("file.exportFrame", &["экспорт кадра", "экспортируй кадр", "export frame"]),
    ("file.newSequence", &["новая секвенция", "новый эпизод", "new sequence"]),
    ("file.newBin", &["новая корзина", "создай папку", "new bin"]),
    ("window.workspace.editing", &["рабочая область монтаж", "editing workspace"]),
    ("window.workspace.color", &["рабочая область цвет", "color workspace"]),
    ("window.workspace.audio", &["рабочая область звук", "audio workspace"]),
    ("window.workspace.effects", &["рабочая область эффекты", "effects workspace"]),
    ("window.workspace.reset", &["сбрось расположение панелей", "reset workspace"]),
    ("window.panel.timeline", &["покажи таймлайн", "show timeline"]),
    ("window.panel.project", &["покажи проект", "show project"]),
    ("window.panel.effects", &["покажи эффекты", "show effects"]),
    ("window.panel.effectControls", &["покажи настройки эффектов", "show effect controls"]),
    ("window.panel.source", &["покажи источник", "show source"]),
    ("window.panel.program", &["покажи программу", "show program"]),
    ("app.settings.general", &["открой настройки", "настройки", "open settings"]),
    ("app.settings.jarvis", &["команды", "список команд", "настройки джарвиса", "voice commands", "jarvis settings"]),
    ("jarvis.cancel", &["закрой диалог", "отмени диалог", "close dialog", "cancel dialog"]),
];

#[derive(Debug, PartialEq)]
pub(super) struct Intent {
    pub id: &'static str,
    pub params: Value,
}
impl Intent {
    fn new(id: &'static str, params: Value) -> Self {
        Self { id, params }
    }
}

/// The catalog comes from the same registries as menus and shortcuts. Internal parameter-only
/// APIs are not promoted to spoken commands; UI dialogs still collect their own parameters.
pub(super) struct Entry {
    pub id: &'static str,
    pub phrases: Vec<String>,
}
pub(super) fn catalog() -> &'static [Entry] {
    static CATALOG: OnceLock<Vec<Entry>> = OnceLock::new();
    CATALOG.get_or_init(|| {
        let mut out: Vec<Entry> = Vec::new();
        let mut add = |id: &'static str, label: &str, menu: &[&str]| {
            let mut phrases = vec![normalize(label), normalize(Language::Ru.tr(label))];
            if !menu.is_empty() {
                for lang in [Language::En, Language::Ru] {
                    let path = menu.iter().copied().chain(std::iter::once(label)).map(|s| lang.tr(s)).collect::<Vec<_>>().join(" ");
                    phrases.push(normalize(&path));
                }
            }
            phrases.sort();
            phrases.dedup();
            out.push(Entry { id, phrases });
        };
        for c in filmcraft_engine::command_specs() {
            if !c.menu.is_empty() || (c.params == "{}" && c.shortcut.is_some()) {
                add(c.id, c.label, c.menu);
            }
        }
        for c in menus::UI_COMMANDS.iter().chain(crate::panels::keyboard::COMMANDS) {
            if !c.menu.is_empty() || c.shortcut.is_some() {
                add(c.id, c.label, c.menu);
            }
        }
        out
    })
}

fn unique(mut ids: impl Iterator<Item = &'static str>) -> Option<&'static str> {
    let first = ids.next()?;
    ids.all(|id| id == first).then_some(first)
}
fn aliases(phrase: &str, fuzzy: bool) -> Option<&'static str> {
    unique(COMMANDS.iter().filter(|(_, aa)| aa.iter().any(|a| if fuzzy { near_phrase(phrase, a) } else { phrase == *a })).map(|(id, _)| *id))
}
#[cfg(test)]
pub(super) fn command(text: &str) -> Option<&'static str> {
    parse(text).map(|c| c.id)
}

pub(super) fn parse(text: &str) -> Option<Intent> {
    conversation::parse(text)
}
fn parse_single(text: &str) -> Option<Intent> {
    if text.chars().count() > 512 || text.chars().zip(text.chars().skip(1)).any(|(a, b)| matches!(a, '-' | '−') && b.is_ascii_digit()) {
        return None;
    }
    let normalized = normalize(text);
    let words: Vec<_> = normalized.split_whitespace().collect();
    if words.len() > 32 || words.iter().any(|w| matches!(*w, "не" | "нет" | "not" | "dont" | "don't" | "never" | "нельзя" | "отмена")) {
        return None;
    }
    // Only remove politeness, never semantic words like "clip" (Mark Clip vs Mark Selection).
    let phrase = words.into_iter().filter(|w| !matches!(*w, "пожалуйста" | "please" | "давай" | "ну" | "ка")).collect::<Vec<_>>().join(" ");
    if phrase.is_empty() {
        return None;
    }
    if let Some(intent) = numeric(&phrase) {
        return Some(intent);
    }
    if let Some(intent) = super::workflows::parse(&phrase) {
        return Some(intent);
    }
    // Exact matches always win over approximate transcription corrections.
    if let Some(id) = aliases(&phrase, false) {
        return Some(Intent::new(id, json!({})));
    }
    if let Some(intent) = natural::parse(&phrase) {
        return Some(intent);
    }
    // Names from menus can be qualified by their full menu path to resolve ambiguity.
    let menu_phrase = phrase.strip_prefix("команда ").or_else(|| phrase.strip_prefix("command ")).unwrap_or(&phrase);
    let matches: Vec<_> = catalog().iter().filter(|e| e.phrases.iter().any(|p| p == menu_phrase)).map(|e| e.id).collect();
    if !matches.is_empty() {
        return unique(matches.into_iter()).map(|id| Intent::new(id, json!({})));
    }
    for suffix in [
        " this clip",
        " the clip",
        " selected clips",
        " этот клип",
        " выделенный клип",
        " clip",
        " clips",
        " клип",
        " клипы",
        " выделенное",
        " выделенные клипы",
        " it",
    ] {
        if let Some(base) = phrase.strip_suffix(suffix)
            && let Some(id) = aliases(base, false)
            && matches!(id, "edit.copy" | "edit.cut" | "edit.paste" | "edit.clear" | "edit.rippleDelete" | "sequence.addEdit")
        {
            return Some(Intent::new(id, json!({})));
        }
    }
    aliases(&phrase, true).map(|id| Intent::new(id, json!({})))
}

// Intentionally bounded integers: never infer units, truncate unknown suffixes or fuzzy-match
// parameterized commands. Decimal/timecode/free-form field dictation is not supported here.
pub(super) fn number(s: &str) -> Option<i64> {
    if let Some((tens, units)) = s.split_once(' ') {
        if units.contains(' ') {
            return None;
        }
        let a = number(tens)?;
        let b = number(units)?;
        return ((20..=90).contains(&a) && a % 10 == 0 && (1..=9).contains(&b)).then_some(a + b);
    }
    match s {
        "ноль" | "zero" => Some(0),
        "один" | "одна" | "one" => Some(1),
        "два" | "две" | "two" => Some(2),
        "три" | "three" => Some(3),
        "четыре" | "four" => Some(4),
        "пять" | "five" => Some(5),
        "шесть" | "six" => Some(6),
        "семь" | "seven" => Some(7),
        "восемь" | "eight" => Some(8),
        "девять" | "nine" => Some(9),
        "десять" | "ten" => Some(10),
        "одиннадцать" | "eleven" => Some(11),
        "двенадцать" | "twelve" => Some(12),
        "тринадцать" | "thirteen" => Some(13),
        "четырнадцать" | "fourteen" => Some(14),
        "пятнадцать" | "fifteen" => Some(15),
        "шестнадцать" | "sixteen" => Some(16),
        "семнадцать" | "seventeen" => Some(17),
        "восемнадцать" | "eighteen" => Some(18),
        "девятнадцать" | "nineteen" => Some(19),
        "двадцать" | "twenty" => Some(20),
        "тридцать" | "thirty" => Some(30),
        "сорок" | "forty" => Some(40),
        "шестьдесят" | "sixty" => Some(60),
        "семьдесят" | "seventy" => Some(70),
        "восемьдесят" | "eighty" => Some(80),
        "девяносто" | "ninety" => Some(90),
        "пятьдесят" | "fifty" => Some(50),
        "сто" | "hundred" => Some(100),
        _ => s.parse::<i64>().ok(),
    }
}
fn numeric(s: &str) -> Option<Intent> {
    let w: Vec<_> = s.split_whitespace().collect();
    if let [direction @ ("вперед" | "назад" | "громче" | "тише"), "на", n, unit] = w.as_slice() {
        return numeric(&format!("{direction} {n} {unit}"));
    }
    match w.as_slice() {
        [direction @ ("вперед" | "назад" | "forward" | "back"), n, unit @ ("кадр" | "кадра" | "кадров" | "frame" | "frames")] => {
            let _ = unit;
            let n = number(n).filter(|n| (1..=10000).contains(n))?;
            Some(Intent::new("playhead.step", json!({"frames": if matches!(*direction, "назад" | "back") { -n } else { n }})))
        }
        ["на", n, "секунду"] | ["перейди", "на", n, "секунду"] | ["go", "to", n, "seconds"] => {
            let n = number(n).filter(|n| (0..=86400).contains(n))?;
            Some(Intent::new("playhead.set", json!({"seconds": n})))
        }
        ["скорость" | "speed", n, "процентов" | "процент" | "процента" | "percent"] => {
            let n = number(n).filter(|n| (1..=1000).contains(n))?;
            Some(Intent::new("clip.speedDuration", json!({"speed": n})))
        }
        [direction @ ("громче" | "тише" | "louder" | "quieter"), n, "дб" | "децибел" | "децибела" | "db"] => {
            let n = number(n).filter(|n| (1..=24).contains(n))?;
            Some(Intent::new("clip.audioGain", json!({"mode": "adjust", "db": if matches!(*direction, "тише" | "quieter") { -n } else { n }})))
        }
        [action, "дорожку" | "track", kind, n] => {
            let (key, value) = match *action {
                "заблокируй" | "lock" => ("locked", true),
                "разблокируй" | "unlock" => ("locked", false),
                "заглуши" | "mute" => ("muted", true),
                "включи" | "unmute" => ("muted", false),
                _ => return None,
            };
            let kind = match *kind {
                "видео" | "video" => "V",
                "аудио" | "audio" => "A",
                _ => return None,
            };
            if key == "muted" && kind != "A" {
                return None;
            }
            let n = number(n).filter(|n| (1..=99).contains(n))?;
            Some(Intent::new("timeline.setTrack", json!({"track": format!("{kind}{n}"), key: value})))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn every_alias_has_a_real_command_and_exact_match() {
        for (id, aa) in COMMANDS {
            assert!(
                id.starts_with("jarvis.")
                    || filmcraft_engine::find_command(id).is_some()
                    || menus::UI_COMMANDS.iter().any(|c| c.id == *id)
                    || id.strip_prefix("window.panel.").is_some_and(|p| crate::dock::PanelKind::from_name(p).is_some()),
                "{id}"
            );
            for a in *aa {
                assert_eq!(command(a), Some(*id), "{a}");
            }
        }
        println!("{} curated voice actions; {} menu/shortcut entries", COMMANDS.len(), catalog().len());
    }
    #[test]
    fn parameterized_commands_preserve_numbers_units_and_direction() {
        for (phrase, id, params) in [
            ("вперед на 10 кадров", "playhead.step", json!({"frames":10})),
            ("back five frames", "playhead.step", json!({"frames":-5})),
            ("перейди на 12 секунду", "playhead.set", json!({"seconds":12})),
            ("speed 50 percent", "clip.speedDuration", json!({"speed":50})),
            ("speed 50%", "clip.speedDuration", json!({"speed":50})),
            ("тише на 3 дб", "clip.audioGain", json!({"mode":"adjust","db":-3})),
            ("заблокируй дорожку видео 1", "timeline.setTrack", json!({"track":"V1","locked":true})),
            ("unmute track audio two", "timeline.setTrack", json!({"track":"A2","muted":false})),
        ] {
            assert_eq!(parse(phrase), Some(Intent::new(id, params)), "{phrase}");
        }
        for phrase in [
            "speed -50 percent",
            "speed 0 percent",
            "speed 999999999999999999999999 percent",
            "speed fifty bananas",
            "вперед 2.5 кадров",
            "вперед 99999 кадров",
            "вперед 10 кадров и удали",
            "mute track video 1",
            "lock track video 0",
            "louder 100 db",
        ] {
            assert_eq!(parse(phrase), None, "{phrase}");
        }
    }
    #[test]
    fn semantic_clip_words_and_negatives_are_not_discarded() {
        for (phrase, id) in [
            ("mark clip", "markers.markClip"),
            ("mark selection", "markers.markSelection"),
            ("copy this clip", "edit.copy"),
            ("please select next clip", "timeline.selectNextClip"),
            ("отмени", "edit.undo"),
        ] {
            assert_eq!(command(phrase), Some(id), "{phrase}");
        }
        for phrase in [
            "не сохраняй",
            "don't select all",
            "do not copy",
            "never delete",
            "copy then delete",
            "удали проект",
            "delete all",
            "отмена",
            "не надо удалять",
            "я сказал скопируй",
        ] {
            assert_eq!(command(phrase), None, "{phrase}");
        }
    }
    #[test]
    fn unique_menu_paths_resolve_in_both_languages() {
        let mut tested = 0;
        for entry in catalog() {
            for phrase in &entry.phrases {
                let ids: Vec<_> = catalog().iter().filter(|e| e.phrases.contains(phrase)).map(|e| e.id).collect();
                let expected = unique(ids.into_iter());
                // Explicit "command" bypasses conversational aliases, while ambiguity stays rejected.
                assert_eq!(command(&format!("command {phrase}")), expected, "{}: {phrase}", entry.id);
                tested += 1;
            }
        }
        println!("{tested} bilingual registry phrases verified");
    }
}
