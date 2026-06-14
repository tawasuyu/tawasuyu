//! Brazos del dispatch `decl_kind_from_pair` — grupo dispatch_c.
//! Extraído de la mega-función original; se mantiene el orden exacto de
//! los brazos (props únicas) para preservar el comportamiento.
use super::*;

pub(crate) fn dispatch_c(p: &str, value: &str) -> Option<DeclKind> {
    match p {
        // `clip` (CSS2.1 §11.1.2, deprecada): `auto | rect(...)`. NO hereda.
        "clip" => parse_clip(value).map(DeclKind::Clip),
        // Fase 7.566 — `elevation` (CSS 2.1 aural). Parse opaco; `level` → None.
        "elevation" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("level") {
                Some(DeclKind::Elevation(None))
            } else {
                Some(DeclKind::Elevation(Some(v.to_string())))
            }
        }
        // Fase 7.567 — `richness` (CSS 2.1 aural). Número 0–100, clamp.
        "richness" => value
            .trim()
            .parse::<f32>()
            .ok()
            .map(|n| DeclKind::Richness(n.clamp(0.0, 100.0))),
        // Fase 7.568 — `stress` (CSS 2.1 aural). Número 0–100, clamp.
        "stress" => value
            .trim()
            .parse::<f32>()
            .ok()
            .map(|n| DeclKind::Stress(n.clamp(0.0, 100.0))),
        // Fase 7.569-7.571 — `pitch`/`speech-rate`/`volume` (CSS 2.1 aural).
        // Parse opaco; `medium` → None.
        "pitch" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("medium") {
                Some(DeclKind::Pitch(None))
            } else {
                Some(DeclKind::Pitch(Some(v.to_string())))
            }
        }
        "speech-rate" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("medium") {
                Some(DeclKind::SpeechRate(None))
            } else {
                Some(DeclKind::SpeechRate(Some(v.to_string())))
            }
        }
        "volume" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("medium") {
                Some(DeclKind::Volume(None))
            } else {
                Some(DeclKind::Volume(Some(v.to_string())))
            }
        }
        // Fase 7.572 — `speak` (CSS 2.1 aural). Distinto de `speak-as`.
        "speak" => match value.trim().to_ascii_lowercase().as_str() {
            "normal" => Some(DeclKind::Speak(Speak::Normal)),
            "none" => Some(DeclKind::Speak(Speak::None)),
            "spell-out" => Some(DeclKind::Speak(Speak::SpellOut)),
            // Fase 7.927 — redefinición CSS Speech 1: `auto | never | always`.
            "auto" => Some(DeclKind::Speak(Speak::Auto)),
            "never" => Some(DeclKind::Speak(Speak::Never)),
            "always" => Some(DeclKind::Speak(Speak::Always)),
            _ => None,
        },
        // Fase 7.573 — `play-during` (CSS 2.1 aural). Parse opaco; `auto` → None.
        "play-during" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("auto") {
                Some(DeclKind::PlayDuring(None))
            } else {
                Some(DeclKind::PlayDuring(Some(v.to_string())))
            }
        }
        // Fase 7.574 — `text-decoration-skip` (CSS Text Decor 4, shorthand
        // legacy). Parse opaco; `auto` → None.
        // Fase 7.743 — alias `-webkit-text-decoration-skip` → estándar.
        "text-decoration-skip" | "-webkit-text-decoration-skip" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("auto") {
                Some(DeclKind::TextDecorationSkip(None))
            } else {
                Some(DeclKind::TextDecorationSkip(Some(v.to_string())))
            }
        }
        // Fase 7.575 — `text-decoration-skip-box` (CSS Text Decor 4).
        "text-decoration-skip-box" => match value.trim().to_ascii_lowercase().as_str() {
            "none" => Some(DeclKind::TextDecorationSkipBox(TextDecorationSkipBox::None)),
            "all" => Some(DeclKind::TextDecorationSkipBox(TextDecorationSkipBox::All)),
            _ => None,
        },
        // Fase 7.576 — `text-decoration-skip-self` (CSS Text Decor 4).
        "text-decoration-skip-self" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("auto") {
                Some(DeclKind::TextDecorationSkipSelf(None))
            } else {
                Some(DeclKind::TextDecorationSkipSelf(Some(v.to_string())))
            }
        }
        // Fase 7.577 — `text-decoration-skip-spaces` (CSS Text Decor 4).
        // `start end` (default) → None.
        "text-decoration-skip-spaces" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("start end") {
                Some(DeclKind::TextDecorationSkipSpaces(None))
            } else {
                Some(DeclKind::TextDecorationSkipSpaces(Some(v.to_string())))
            }
        }
        // Fase 7.578 — `text-decoration-skip-inset` (CSS Text Decor 4).
        "text-decoration-skip-inset" => match value.trim().to_ascii_lowercase().as_str() {
            "none" => Some(DeclKind::TextDecorationSkipInset(TextDecorationSkipInset::None)),
            "auto" => Some(DeclKind::TextDecorationSkipInset(TextDecorationSkipInset::Auto)),
            _ => None,
        },
        // Fase 7.579 — `-webkit-text-stroke-width`. px o número desnudo;
        // keywords thin/medium/thick → 1/3/5.
        "-webkit-text-stroke-width" => {
            let v = value.trim().to_ascii_lowercase();
            match v.as_str() {
                "thin" => Some(DeclKind::WebkitTextStrokeWidth(1.0)),
                "medium" => Some(DeclKind::WebkitTextStrokeWidth(3.0)),
                "thick" => Some(DeclKind::WebkitTextStrokeWidth(5.0)),
                _ => {
                    let num = v.strip_suffix("px").unwrap_or(&v);
                    num.parse::<f32>().ok().map(DeclKind::WebkitTextStrokeWidth)
                }
            }
        }
        // Fase 7.580-7.581 — `-webkit-text-{stroke,fill}-color`. Parse opaco;
        // `currentcolor` → None.
        "-webkit-text-stroke-color" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("currentcolor") {
                Some(DeclKind::WebkitTextStrokeColor(None))
            } else {
                Some(DeclKind::WebkitTextStrokeColor(Some(v.to_string())))
            }
        }
        "-webkit-text-fill-color" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("currentcolor") {
                Some(DeclKind::WebkitTextFillColor(None))
            } else {
                Some(DeclKind::WebkitTextFillColor(Some(v.to_string())))
            }
        }
        // Fase 7.582 — `font-smooth` (no estándar). Parse opaco; `auto` → None.
        "font-smooth" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("auto") {
                Some(DeclKind::FontSmooth(None))
            } else {
                Some(DeclKind::FontSmooth(Some(v.to_string())))
            }
        }
        // Fase 7.583 — `text-group-align` (CSS Text 4).
        "text-group-align" => match value.trim().to_ascii_lowercase().as_str() {
            "none" => Some(DeclKind::TextGroupAlign(TextGroupAlign::None)),
            "start" => Some(DeclKind::TextGroupAlign(TextGroupAlign::Start)),
            "end" => Some(DeclKind::TextGroupAlign(TextGroupAlign::End)),
            "left" => Some(DeclKind::TextGroupAlign(TextGroupAlign::Left)),
            "right" => Some(DeclKind::TextGroupAlign(TextGroupAlign::Right)),
            "center" => Some(DeclKind::TextGroupAlign(TextGroupAlign::Center)),
            _ => None,
        },
        // Fase 7.584 — `continue` (CSS Overflow 4).
        "continue" => match value.trim().to_ascii_lowercase().as_str() {
            "auto" => Some(DeclKind::Continue(Continue::Auto)),
            "discard" => Some(DeclKind::Continue(Continue::Discard)),
            _ => None,
        },
        // Fase 7.585 — `block-ellipsis` (CSS Overflow 4). Parse opaco;
        // `none` → None (también `auto` se conserva como string).
        "block-ellipsis" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("none") {
                Some(DeclKind::BlockEllipsis(None))
            } else {
                Some(DeclKind::BlockEllipsis(Some(v.to_string())))
            }
        }
        // Fase 7.586 — `max-lines` (CSS Overflow 4). `none` → None.
        "max-lines" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("none") {
                Some(DeclKind::MaxLines(None))
            } else {
                v.parse::<u32>().ok().map(|n| DeclKind::MaxLines(Some(n)))
            }
        }
        // Fase 7.587 — `region-fragment` (CSS Regions 1).
        "region-fragment" => match value.trim().to_ascii_lowercase().as_str() {
            "auto" => Some(DeclKind::RegionFragment(RegionFragment::Auto)),
            "break" => Some(DeclKind::RegionFragment(RegionFragment::Break)),
            _ => None,
        },
        // Fase 7.588 — `overflow-style` (CSS Marquee/Basic UI legacy).
        // Parse opaco; `auto` → None.
        "overflow-style" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("auto") {
                Some(DeclKind::OverflowStyle(None))
            } else {
                Some(DeclKind::OverflowStyle(Some(v.to_string())))
            }
        }
        // Fase 7.589 — `marquee-style` (CSS Marquee).
        "marquee-style" => match value.trim().to_ascii_lowercase().as_str() {
            "scroll" => Some(DeclKind::MarqueeStyle(MarqueeStyle::Scroll)),
            "slide" => Some(DeclKind::MarqueeStyle(MarqueeStyle::Slide)),
            "alternate" => Some(DeclKind::MarqueeStyle(MarqueeStyle::Alternate)),
            _ => None,
        },
        // Fase 7.590 — `marquee-direction` (CSS Marquee).
        "marquee-direction" => match value.trim().to_ascii_lowercase().as_str() {
            "forward" => Some(DeclKind::MarqueeDirection(MarqueeDirection::Forward)),
            "reverse" => Some(DeclKind::MarqueeDirection(MarqueeDirection::Reverse)),
            _ => None,
        },
        // Fase 7.591 — `marquee-speed` (CSS Marquee).
        "marquee-speed" => match value.trim().to_ascii_lowercase().as_str() {
            "slow" => Some(DeclKind::MarqueeSpeed(MarqueeSpeed::Slow)),
            "normal" => Some(DeclKind::MarqueeSpeed(MarqueeSpeed::Normal)),
            "fast" => Some(DeclKind::MarqueeSpeed(MarqueeSpeed::Fast)),
            _ => None,
        },
        // Fase 7.592 — `marquee-loop` (CSS Marquee). `infinite` → None.
        "marquee-loop" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("infinite") {
                Some(DeclKind::MarqueeLoop(None))
            } else {
                v.parse::<i32>().ok().map(|n| DeclKind::MarqueeLoop(Some(n)))
            }
        }
        // Fase 7.593 — `marquee-increment` (CSS Marquee). `6px` (default) → None.
        "marquee-increment" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("6px") {
                Some(DeclKind::MarqueeIncrement(None))
            } else {
                Some(DeclKind::MarqueeIncrement(Some(v.to_string())))
            }
        }
        // Fase 7.594-7.598 — familia `nav-*` (CSS UI 3 legacy). Parse opaco;
        // `auto` → None.
        "nav-index" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("auto") {
                Some(DeclKind::NavIndex(None))
            } else {
                Some(DeclKind::NavIndex(Some(v.to_string())))
            }
        }
        "nav-up" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("auto") {
                Some(DeclKind::NavUp(None))
            } else {
                Some(DeclKind::NavUp(Some(v.to_string())))
            }
        }
        "nav-down" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("auto") {
                Some(DeclKind::NavDown(None))
            } else {
                Some(DeclKind::NavDown(Some(v.to_string())))
            }
        }
        "nav-left" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("auto") {
                Some(DeclKind::NavLeft(None))
            } else {
                Some(DeclKind::NavLeft(Some(v.to_string())))
            }
        }
        "nav-right" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("auto") {
                Some(DeclKind::NavRight(None))
            } else {
                Some(DeclKind::NavRight(Some(v.to_string())))
            }
        }
        // Fase 7.599-7.602 — `-webkit-box-{orient,direction,align,pack}`
        // (flexbox viejo). Parse opaco; sentinel default → None.
        // Fase 7.784 — `-moz-box-orient` (XUL flexbox, mismo semántico que -webkit-).
        "-webkit-box-orient" | "-moz-box-orient" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("inline-axis") {
                Some(DeclKind::WebkitBoxOrient(None))
            } else {
                Some(DeclKind::WebkitBoxOrient(Some(v.to_string())))
            }
        }
        // Fase 7.785 — `-moz-box-direction`.
        "-webkit-box-direction" | "-moz-box-direction" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("normal") {
                Some(DeclKind::WebkitBoxDirection(None))
            } else {
                Some(DeclKind::WebkitBoxDirection(Some(v.to_string())))
            }
        }
        // Fase 7.786 — `-moz-box-align`.
        "-webkit-box-align" | "-moz-box-align" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("stretch") {
                Some(DeclKind::WebkitBoxAlign(None))
            } else {
                Some(DeclKind::WebkitBoxAlign(Some(v.to_string())))
            }
        }
        // Fase 7.787 — `-moz-box-pack`.
        "-webkit-box-pack" | "-moz-box-pack" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("start") {
                Some(DeclKind::WebkitBoxPack(None))
            } else {
                Some(DeclKind::WebkitBoxPack(Some(v.to_string())))
            }
        }
        // Fase 7.603 — `-webkit-box-flex` / Fase 7.788 — `-moz-box-flex` (flexbox viejo). Número desnudo.
        "-webkit-box-flex" | "-moz-box-flex" => value
            .trim()
            .parse::<f32>()
            .ok()
            .map(DeclKind::WebkitBoxFlex),
        // Fase 7.604 — `-webkit-box-ordinal-group` / Fase 7.789 — `-moz-box-ordinal-group` (flexbox viejo). `1` → None.
        "-webkit-box-ordinal-group" | "-moz-box-ordinal-group" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v == "1" {
                Some(DeclKind::WebkitBoxOrdinalGroup(None))
            } else {
                v.parse::<u32>().ok().map(|n| DeclKind::WebkitBoxOrdinalGroup(Some(n)))
            }
        }
        // Fase 7.605-7.606 — `-webkit-font-smoothing` / `-moz-osx-font-smoothing`
        // (no estándar). Parse opaco; `auto` → None.
        "-webkit-font-smoothing" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("auto") {
                Some(DeclKind::WebkitFontSmoothing(None))
            } else {
                Some(DeclKind::WebkitFontSmoothing(Some(v.to_string())))
            }
        }
        "-moz-osx-font-smoothing" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("auto") {
                Some(DeclKind::MozOsxFontSmoothing(None))
            } else {
                Some(DeclKind::MozOsxFontSmoothing(Some(v.to_string())))
            }
        }
        // Fase 7.607 — `-webkit-tap-highlight-color`. Parse opaco.
        "-webkit-tap-highlight-color" => {
            let v = value.trim();
            if v.is_empty() { None }
            else { Some(DeclKind::WebkitTapHighlightColor(Some(v.to_string()))) }
        }
        // Fase 7.608 — `zoom`. Parse opaco; `normal` → None.
        "zoom" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("normal") {
                Some(DeclKind::Zoom(None))
            } else {
                Some(DeclKind::Zoom(Some(v.to_string())))
            }
        }
        // Fase 7.614-7.616 — `column-break-{before,after,inside}` (Multicol
        // legacy). Parse opaco; `auto` → None.
        "column-break-before" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("auto") {
                Some(DeclKind::ColumnBreakBefore(None))
            } else {
                Some(DeclKind::ColumnBreakBefore(Some(v.to_string())))
            }
        }
        "column-break-after" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("auto") {
                Some(DeclKind::ColumnBreakAfter(None))
            } else {
                Some(DeclKind::ColumnBreakAfter(Some(v.to_string())))
            }
        }
        "column-break-inside" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("auto") {
                Some(DeclKind::ColumnBreakInside(None))
            } else {
                Some(DeclKind::ColumnBreakInside(Some(v.to_string())))
            }
        }
        // Fase 7.617 — `user-modify` (+ alias `-webkit-user-modify`). `read-only` → None.
        // Fase 7.810 — `-moz-user-modify` alias vendor (Gecko, mismo semántico).
        "user-modify" | "-webkit-user-modify" | "-moz-user-modify" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("read-only") {
                Some(DeclKind::UserModify(None))
            } else {
                Some(DeclKind::UserModify(Some(v.to_string())))
            }
        }
        // Fase 7.618 — `-webkit-touch-callout` (iOS). `default` → None.
        "-webkit-touch-callout" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("default") {
                Some(DeclKind::WebkitTouchCallout(None))
            } else {
                Some(DeclKind::WebkitTouchCallout(Some(v.to_string())))
            }
        }
        // Fase 7.619 — `-webkit-user-drag`. `auto` → None.
        "-webkit-user-drag" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("auto") {
                Some(DeclKind::WebkitUserDrag(None))
            } else {
                Some(DeclKind::WebkitUserDrag(Some(v.to_string())))
            }
        }
        // Fase 7.620 — `-webkit-rtl-ordering`. `logical` → None.
        "-webkit-rtl-ordering" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("logical") {
                Some(DeclKind::WebkitRtlOrdering(None))
            } else {
                Some(DeclKind::WebkitRtlOrdering(Some(v.to_string())))
            }
        }
        // Fase 7.621 — `-webkit-text-security`. `none` → None.
        "-webkit-text-security" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("none") {
                Some(DeclKind::WebkitTextSecurity(None))
            } else {
                Some(DeclKind::WebkitTextSecurity(Some(v.to_string())))
            }
        }
        // Fase 7.622 — `-webkit-nbsp-mode`. `normal` → None.
        "-webkit-nbsp-mode" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("normal") {
                Some(DeclKind::WebkitNbspMode(None))
            } else {
                Some(DeclKind::WebkitNbspMode(Some(v.to_string())))
            }
        }
        // Fase 7.623 — `-webkit-locale`. `auto` → None.
        "-webkit-locale" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("auto") {
                Some(DeclKind::WebkitLocale(None))
            } else {
                Some(DeclKind::WebkitLocale(Some(v.to_string())))
            }
        }
        // Fase 7.624 — `-webkit-column-axis`. `auto` → None.
        "-webkit-column-axis" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("auto") {
                Some(DeclKind::WebkitColumnAxis(None))
            } else {
                Some(DeclKind::WebkitColumnAxis(Some(v.to_string())))
            }
        }
        // Fase 7.625 — `-webkit-column-progression`. `normal` → None.
        "-webkit-column-progression" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("normal") {
                Some(DeclKind::WebkitColumnProgression(None))
            } else {
                Some(DeclKind::WebkitColumnProgression(Some(v.to_string())))
            }
        }
        // Fase 7.626 — `-webkit-app-region` (Chrome/Electron). `none` → None.
        "-webkit-app-region" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("none") {
                Some(DeclKind::WebkitAppRegion(None))
            } else {
                Some(DeclKind::WebkitAppRegion(Some(v.to_string())))
            }
        }
        // Fase 7.627 — `-webkit-highlight`. `none` → None.
        "-webkit-highlight" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("none") {
                Some(DeclKind::WebkitHighlight(None))
            } else {
                Some(DeclKind::WebkitHighlight(Some(v.to_string())))
            }
        }
        // Fase 7.628 — `-webkit-box-reflect`. `none` → None.
        "-webkit-box-reflect" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("none") {
                Some(DeclKind::WebkitBoxReflect(None))
            } else {
                Some(DeclKind::WebkitBoxReflect(Some(v.to_string())))
            }
        }
        // Fase 7.644 — `-webkit-mask-composite`. `add` → None.
        "-webkit-mask-composite" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("add") {
                Some(DeclKind::WebkitMaskComposite(None))
            } else {
                Some(DeclKind::WebkitMaskComposite(Some(v.to_string())))
            }
        }
        // Fase 7.645 — `-webkit-mask-position-x`. `center` → None.
        "-webkit-mask-position-x" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("center") {
                Some(DeclKind::WebkitMaskPositionX(None))
            } else {
                Some(DeclKind::WebkitMaskPositionX(Some(v.to_string())))
            }
        }
        // Fase 7.646 — `-webkit-mask-position-y`. `center` → None.
        "-webkit-mask-position-y" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("center") {
                Some(DeclKind::WebkitMaskPositionY(None))
            } else {
                Some(DeclKind::WebkitMaskPositionY(Some(v.to_string())))
            }
        }
        // Fase 7.647 — `-webkit-mask-repeat-x`. `repeat` → None.
        "-webkit-mask-repeat-x" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("repeat") {
                Some(DeclKind::WebkitMaskRepeatX(None))
            } else {
                Some(DeclKind::WebkitMaskRepeatX(Some(v.to_string())))
            }
        }
        // Fase 7.648 — `-webkit-mask-repeat-y`. `repeat` → None.
        "-webkit-mask-repeat-y" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("repeat") {
                Some(DeclKind::WebkitMaskRepeatY(None))
            } else {
                Some(DeclKind::WebkitMaskRepeatY(Some(v.to_string())))
            }
        }
        // Fase 7.649 — `-webkit-margin-start` (alias legacy de
        // margin-inline-start). `0` → None.
        // Fase 7.806 — `-moz-margin-start` alias vendor (Gecko, mismo semántico).
        "-webkit-margin-start" | "-moz-margin-start" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v == "0" {
                Some(DeclKind::WebkitMarginStart(None))
            } else {
                Some(DeclKind::WebkitMarginStart(Some(v.to_string())))
            }
        }
        // Fase 7.650 — `-webkit-margin-end` / Fase 7.807 — `-moz-margin-end`. `0` → None.
        "-webkit-margin-end" | "-moz-margin-end" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v == "0" {
                Some(DeclKind::WebkitMarginEnd(None))
            } else {
                Some(DeclKind::WebkitMarginEnd(Some(v.to_string())))
            }
        }
        // Fase 7.651 — `-webkit-margin-before` (block-start). `0` → None.
        "-webkit-margin-before" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v == "0" {
                Some(DeclKind::WebkitMarginBefore(None))
            } else {
                Some(DeclKind::WebkitMarginBefore(Some(v.to_string())))
            }
        }
        // Fase 7.652 — `-webkit-margin-after` (block-end). `0` → None.
        "-webkit-margin-after" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v == "0" {
                Some(DeclKind::WebkitMarginAfter(None))
            } else {
                Some(DeclKind::WebkitMarginAfter(Some(v.to_string())))
            }
        }
        // Fase 7.653 — `-webkit-padding-start`. `0` → None.
        // Fase 7.808 — `-moz-padding-start` alias vendor (Gecko, mismo semántico).
        "-webkit-padding-start" | "-moz-padding-start" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v == "0" {
                Some(DeclKind::WebkitPaddingStart(None))
            } else {
                Some(DeclKind::WebkitPaddingStart(Some(v.to_string())))
            }
        }
        // Fase 7.654 — `-webkit-padding-end` / Fase 7.809 — `-moz-padding-end`. `0` → None.
        "-webkit-padding-end" | "-moz-padding-end" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v == "0" {
                Some(DeclKind::WebkitPaddingEnd(None))
            } else {
                Some(DeclKind::WebkitPaddingEnd(Some(v.to_string())))
            }
        }
        // Fase 7.655 — `-webkit-padding-before` (block-start). `0` → None.
        "-webkit-padding-before" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v == "0" {
                Some(DeclKind::WebkitPaddingBefore(None))
            } else {
                Some(DeclKind::WebkitPaddingBefore(Some(v.to_string())))
            }
        }
        // Fase 7.656 — `-webkit-padding-after` (block-end). `0` → None.
        "-webkit-padding-after" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v == "0" {
                Some(DeclKind::WebkitPaddingAfter(None))
            } else {
                Some(DeclKind::WebkitPaddingAfter(Some(v.to_string())))
            }
        }
        // Fase 7.657 — `-webkit-logical-width` (inline-size). `auto` → None.
        "-webkit-logical-width" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("auto") {
                Some(DeclKind::WebkitLogicalWidth(None))
            } else {
                Some(DeclKind::WebkitLogicalWidth(Some(v.to_string())))
            }
        }
        // Fase 7.658 — `-webkit-logical-height` (block-size). `auto` → None.
        "-webkit-logical-height" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("auto") {
                Some(DeclKind::WebkitLogicalHeight(None))
            } else {
                Some(DeclKind::WebkitLogicalHeight(Some(v.to_string())))
            }
        }
        // Fase 7.664 — `-webkit-transform-origin-x`. `50%`/`center` → None.
        "-webkit-transform-origin-x" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v == "50%" || v.eq_ignore_ascii_case("center") {
                Some(DeclKind::WebkitTransformOriginX(None))
            } else {
                Some(DeclKind::WebkitTransformOriginX(Some(v.to_string())))
            }
        }
        // Fase 7.665 — `-webkit-transform-origin-y`. `50%`/`center` → None.
        "-webkit-transform-origin-y" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v == "50%" || v.eq_ignore_ascii_case("center") {
                Some(DeclKind::WebkitTransformOriginY(None))
            } else {
                Some(DeclKind::WebkitTransformOriginY(Some(v.to_string())))
            }
        }
        // Fase 7.666 — `-webkit-transform-origin-z`. `0` → None.
        "-webkit-transform-origin-z" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v == "0" {
                Some(DeclKind::WebkitTransformOriginZ(None))
            } else {
                Some(DeclKind::WebkitTransformOriginZ(Some(v.to_string())))
            }
        }
        // Fase 7.667 — `-webkit-perspective-origin-x`. `50%`/`center` → None.
        "-webkit-perspective-origin-x" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v == "50%" || v.eq_ignore_ascii_case("center") {
                Some(DeclKind::WebkitPerspectiveOriginX(None))
            } else {
                Some(DeclKind::WebkitPerspectiveOriginX(Some(v.to_string())))
            }
        }
        // Fase 7.668 — `-webkit-perspective-origin-y`. `50%`/`center` → None.
        "-webkit-perspective-origin-y" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v == "50%" || v.eq_ignore_ascii_case("center") {
                Some(DeclKind::WebkitPerspectiveOriginY(None))
            } else {
                Some(DeclKind::WebkitPerspectiveOriginY(Some(v.to_string())))
            }
        }
        _ => None,
    }
}
