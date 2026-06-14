//! Brazos del dispatch `decl_kind_from_pair` — grupo dispatch_b.
//! Extraído de la mega-función original; se mantiene el orden exacto de
//! los brazos (props únicas) para preservar el comportamiento.
use super::*;

pub(crate) fn dispatch_b(p: &str, value: &str) -> Option<DeclKind> {
    match p {
        // Fase 7.489 — `string-set` (CSS GCPM). `none | [<custom-ident>
        // <content-list>]#`. Parse opaco para que un renderer GCPM lo
        // evalúe.
        "string-set" => {
            let v = value.trim();
            if v.is_empty() {
                None
            } else if v.eq_ignore_ascii_case("none") {
                Some(DeclKind::StringSet(None))
            } else {
                Some(DeclKind::StringSet(Some(v.to_string())))
            }
        }
        // Fase 7.490 — `footnote-display` (CSS GCPM 4).
        "footnote-display" => match value.trim().to_ascii_lowercase().as_str() {
            "block" => Some(DeclKind::FootnoteDisplay(FootnoteDisplay::Block)),
            "inline" => Some(DeclKind::FootnoteDisplay(FootnoteDisplay::Inline)),
            "compact" => Some(DeclKind::FootnoteDisplay(FootnoteDisplay::Compact)),
            _ => None,
        },
        // Fase 7.491 — `footnote-policy` (CSS GCPM 4).
        "footnote-policy" => match value.trim().to_ascii_lowercase().as_str() {
            "auto" => Some(DeclKind::FootnotePolicy(FootnotePolicy::Auto)),
            "line" => Some(DeclKind::FootnotePolicy(FootnotePolicy::Line)),
            "block" => Some(DeclKind::FootnotePolicy(FootnotePolicy::Block)),
            _ => None,
        },
        // Fase 7.492 — `marker-knockout-left` (CSS GCPM 4).
        "marker-knockout-left" => match value.trim().to_ascii_lowercase().as_str() {
            "auto" => Some(DeclKind::MarkerKnockoutLeft(MarkerKnockout::Auto)),
            "none" => Some(DeclKind::MarkerKnockoutLeft(MarkerKnockout::None)),
            _ => None,
        },
        // Fase 7.493 — `marker-knockout-right` (CSS GCPM 4).
        "marker-knockout-right" => match value.trim().to_ascii_lowercase().as_str() {
            "auto" => Some(DeclKind::MarkerKnockoutRight(MarkerKnockout::Auto)),
            "none" => Some(DeclKind::MarkerKnockoutRight(MarkerKnockout::None)),
            _ => None,
        },
        // Fase 7.494 — `leading-trim` (CSS Inline 3). HEREDA.
        "leading-trim" => match value.trim().to_ascii_lowercase().as_str() {
            "normal" => Some(DeclKind::LeadingTrim(LeadingTrim::Normal)),
            "start" => Some(DeclKind::LeadingTrim(LeadingTrim::Start)),
            "end" => Some(DeclKind::LeadingTrim(LeadingTrim::End)),
            "both" => Some(DeclKind::LeadingTrim(LeadingTrim::Both)),
            _ => None,
        },
        // Fase 7.495 — `initial-letter-align` (CSS Inline 3). HEREDA.
        "initial-letter-align" => match value.trim().to_ascii_lowercase().as_str() {
            "auto" => Some(DeclKind::InitialLetterAlign(InitialLetterAlign::Auto)),
            "alphabetic" => Some(DeclKind::InitialLetterAlign(InitialLetterAlign::Alphabetic)),
            "hanging" => Some(DeclKind::InitialLetterAlign(InitialLetterAlign::Hanging)),
            "ideographic" => Some(DeclKind::InitialLetterAlign(InitialLetterAlign::Ideographic)),
            "border-box" => Some(DeclKind::InitialLetterAlign(InitialLetterAlign::BorderBox)),
            _ => None,
        },
        // Fase 7.496 — `text-autospace` (CSS Text 4). Parse opaco.
        // `normal` reservado → None.
        "text-autospace" => {
            let v = value.trim();
            if v.is_empty() {
                None
            } else if v.eq_ignore_ascii_case("normal") {
                Some(DeclKind::TextAutospace(None))
            } else {
                Some(DeclKind::TextAutospace(Some(v.to_string())))
            }
        }
        // Fase 7.497 — `white-space-trim` (CSS Text 4). Parse opaco.
        // `none` reservado → None.
        "white-space-trim" => {
            let v = value.trim();
            if v.is_empty() {
                None
            } else if v.eq_ignore_ascii_case("none") {
                Some(DeclKind::WhiteSpaceTrim(None))
            } else {
                Some(DeclKind::WhiteSpaceTrim(Some(v.to_string())))
            }
        }
        // Fase 7.498 — `view-transition-group` (CSS View Transitions 2).
        // `normal | contain | nearest | <custom-ident>`. Parse opaco
        // — `normal` reservado a None.
        "view-transition-group" => {
            let v = value.trim();
            if v.is_empty() {
                None
            } else if v.eq_ignore_ascii_case("normal") {
                Some(DeclKind::ViewTransitionGroup(None))
            } else {
                Some(DeclKind::ViewTransitionGroup(Some(v.to_string())))
            }
        }
        // Fase 7.499 — `inset-area` (CSS Anchor Positioning 1, alias
        // legacy de `position-area`). Parse opaco.
        "inset-area" => {
            let v = value.trim();
            if v.is_empty() {
                None
            } else if v.eq_ignore_ascii_case("none") {
                Some(DeclKind::InsetArea(None))
            } else {
                Some(DeclKind::InsetArea(Some(v.to_string())))
            }
        }
        // Fase 7.500 — `view-transition-image-pair` (CSS View Transitions 2).
        "view-transition-image-pair" => {
            let v = value.trim();
            if v.is_empty() {
                None
            } else if v.eq_ignore_ascii_case("auto") {
                Some(DeclKind::ViewTransitionImagePair(None))
            } else {
                Some(DeclKind::ViewTransitionImagePair(Some(v.to_string())))
            }
        }
        // Fase 7.501 — `animation-trigger` (CSS Animations 2, scroll-
        // driven triggers). Shorthand opaco.
        "animation-trigger" => {
            let v = value.trim();
            if v.is_empty() {
                None
            } else if v.eq_ignore_ascii_case("none") || v.eq_ignore_ascii_case("auto") {
                Some(DeclKind::AnimationTrigger(None))
            } else {
                Some(DeclKind::AnimationTrigger(Some(v.to_string())))
            }
        }
        // Fase 7.502 — `border-image-source` (CSS Backgrounds 3).
        // `none | <image>`. Parse opaco para `<image>` (url/gradient).
        "border-image-source" => {
            let v = value.trim();
            if v.is_empty() {
                None
            } else if v.eq_ignore_ascii_case("none") {
                Some(DeclKind::BorderImageSource(None))
            } else {
                Some(DeclKind::BorderImageSource(Some(v.to_string())))
            }
        }
        // Fase 7.503 — `border-image-repeat`. `[stretch|repeat|round|space]{1,2}`.
        "border-image-repeat" => {
            fn kw(s: &str) -> Option<BorderImageRepeat> {
                match s {
                    "stretch" => Some(BorderImageRepeat::Stretch),
                    "repeat" => Some(BorderImageRepeat::Repeat),
                    "round" => Some(BorderImageRepeat::Round),
                    "space" => Some(BorderImageRepeat::Space),
                    _ => None,
                }
            }
            let lower = value.trim().to_ascii_lowercase();
            let parts: Vec<&str> = lower.split_whitespace().collect();
            match parts.len() {
                1 => kw(parts[0]).map(|h| DeclKind::BorderImageRepeat(h, h)),
                2 => match (kw(parts[0]), kw(parts[1])) {
                    (Some(h), Some(v)) => Some(DeclKind::BorderImageRepeat(h, v)),
                    _ => None,
                },
                _ => None,
            }
        }
        // Fase 7.504 — `border-image-slice`. Parse opaco (`<n-p>{1,4} && fill?`).
        "border-image-slice" => {
            let v = value.trim();
            if v.is_empty() { None }
            else { Some(DeclKind::BorderImageSlice(Some(v.to_string()))) }
        }
        // Fase 7.505 — `border-image-width`. Parse opaco.
        "border-image-width" => {
            let v = value.trim();
            if v.is_empty() { None }
            else { Some(DeclKind::BorderImageWidth(Some(v.to_string()))) }
        }
        // Fase 7.506 — `border-image-outset`. Parse opaco.
        "border-image-outset" => {
            let v = value.trim();
            if v.is_empty() { None }
            else { Some(DeclKind::BorderImageOutset(Some(v.to_string()))) }
        }
        // Fase 7.507 — `border-image` shorthand. Parse opaco.
        // Fase 7.780 — `-moz-border-image` alias vendor legacy.
        "border-image" | "-webkit-border-image" | "-moz-border-image" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("none") {
                Some(DeclKind::BorderImage(None))
            } else {
                Some(DeclKind::BorderImage(Some(v.to_string())))
            }
        }
        // Fase 7.508 — `grid-template-areas`. Parse opaco (lista de strings
        // quoted que un resolver de grid consume).
        "grid-template-areas" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("none") {
                Some(DeclKind::GridTemplateAreas(None))
            } else {
                Some(DeclKind::GridTemplateAreas(Some(v.to_string())))
            }
        }
        // Fase 7.509-7.512 — `grid-{row,column}-{start,end}`. Parse opaco
        // de `<grid-line>` (gramática completa `auto | <ident> | <int> |
        // span ...`). El resolver de grid lo evalúa cuando coloca ítems.
        "grid-row-start" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("auto") { Some(DeclKind::GridRowStart(None)) }
            else { Some(DeclKind::GridRowStart(Some(v.to_string()))) }
        }
        "grid-row-end" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("auto") { Some(DeclKind::GridRowEnd(None)) }
            else { Some(DeclKind::GridRowEnd(Some(v.to_string()))) }
        }
        "grid-column-start" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("auto") { Some(DeclKind::GridColumnStart(None)) }
            else { Some(DeclKind::GridColumnStart(Some(v.to_string()))) }
        }
        "grid-column-end" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("auto") { Some(DeclKind::GridColumnEnd(None)) }
            else { Some(DeclKind::GridColumnEnd(Some(v.to_string()))) }
        }
        // Fase 7.513 — `text-emphasis-skip` (CSS Text Decoration 4). HEREDA.
        "text-emphasis-skip" => match value.trim().to_ascii_lowercase().as_str() {
            "spaces" => Some(DeclKind::TextEmphasisSkip(TextEmphasisSkip::Spaces)),
            "punctuation" => Some(DeclKind::TextEmphasisSkip(TextEmphasisSkip::Punctuation)),
            "symbols" => Some(DeclKind::TextEmphasisSkip(TextEmphasisSkip::Symbols)),
            "narrow" => Some(DeclKind::TextEmphasisSkip(TextEmphasisSkip::Narrow)),
            _ => None,
        },
        // Fase 7.514-7.518 — `animation-*` longhands. Mutación parcial de
        // `s.animation` (Option<AnimationBinding>) — el primer longhand
        // crea la binding con defaults, los siguientes ajustan campos.
        // De una lista separada por coma sólo tomamos el primer item, igual
        // que el shorthand `animation:` ya hace en parser/sheet.rs.
        // Fase 7.735 — alias `-webkit-animation-name` → estándar.
        "animation-name" | "-webkit-animation-name" => {
            let v = first_comma(value.trim());
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("none") {
                Some(DeclKind::AnimationName(None))
            } else {
                Some(DeclKind::AnimationName(Some(v.to_string())))
            }
        }
        // Fase 7.736 — alias `-webkit-animation-duration` → estándar.
        "animation-duration" | "-webkit-animation-duration" => parse_time_seconds(first_comma(value.trim()))
            .map(DeclKind::AnimationDuration),
        // Fase 7.737 — alias `-webkit-animation-timing-function` → estándar.
        // Fase 7.855 — usa `parse_easing` (no `_keyword`): acepta también
        // `cubic-bezier(...)` y `steps(...)`, igual que `transition-timing-function`.
        "animation-timing-function" | "-webkit-animation-timing-function" => {
            parse_easing(&first_comma(value.trim()).to_ascii_lowercase())
                .map(DeclKind::AnimationTimingFunction)
        }
        // Fase 7.738 — alias `-webkit-animation-iteration-count` → estándar.
        "animation-iteration-count" | "-webkit-animation-iteration-count" => {
            let t = first_comma(value.trim());
            if t.eq_ignore_ascii_case("infinite") {
                Some(DeclKind::AnimationIterationCount(AnimationIterations::Infinite))
            } else {
                t.parse::<f32>()
                    .ok()
                    .filter(|n| *n >= 0.0)
                    .map(|n| DeclKind::AnimationIterationCount(AnimationIterations::Count(n)))
            }
        }
        // Fase 7.739 — alias `-webkit-animation-fill-mode` → estándar.
        "animation-fill-mode" | "-webkit-animation-fill-mode" => match first_comma(value.trim()).to_ascii_lowercase().as_str() {
            "none" => Some(DeclKind::AnimationFillMode(AnimationFillMode::None)),
            "forwards" => Some(DeclKind::AnimationFillMode(AnimationFillMode::Forwards)),
            "backwards" => Some(DeclKind::AnimationFillMode(AnimationFillMode::Backwards)),
            "both" => Some(DeclKind::AnimationFillMode(AnimationFillMode::Both)),
            _ => None,
        },
        // Fase 7.816 — `animation-direction` longhand (faltaba; sólo el shorthand
        // `animation` lo clasificaba). Mismo destino `AnimationBinding.direction`.
        // Alias `-webkit-animation-direction`. Toma la 1ª de la lista (first_comma).
        "animation-direction" | "-webkit-animation-direction" => match first_comma(value.trim()).to_ascii_lowercase().as_str() {
            "normal" => Some(DeclKind::AnimationDirection(AnimationDirection::Normal)),
            "reverse" => Some(DeclKind::AnimationDirection(AnimationDirection::Reverse)),
            "alternate" => Some(DeclKind::AnimationDirection(AnimationDirection::Alternate)),
            "alternate-reverse" => {
                Some(DeclKind::AnimationDirection(AnimationDirection::AlternateReverse))
            }
            _ => None,
        },
        // Fase 7.817 — `animation-play-state` longhand. Alias `-webkit-`.
        "animation-play-state" | "-webkit-animation-play-state" => match first_comma(value.trim()).to_ascii_lowercase().as_str() {
            "running" => Some(DeclKind::AnimationPlayState(AnimationPlayState::Running)),
            "paused" => Some(DeclKind::AnimationPlayState(AnimationPlayState::Paused)),
            _ => None,
        },
        // Fase 7.818 — `animation-delay` longhand. Alias `-webkit-`. Segundos.
        "animation-delay" | "-webkit-animation-delay" => {
            parse_time_seconds(first_comma(value.trim())).map(DeclKind::AnimationDelay)
        }
        // `float` (CSS2.1 §9.5 + Logical Properties). `none|left|right|
        // inline-start|inline-end`. NO hereda.
        "float" => match value.trim().to_ascii_lowercase().as_str() {
            "none" => Some(DeclKind::Float(Float::None)),
            "left" => Some(DeclKind::Float(Float::Left)),
            "right" => Some(DeclKind::Float(Float::Right)),
            "inline-start" => Some(DeclKind::Float(Float::InlineStart)),
            "inline-end" => Some(DeclKind::Float(Float::InlineEnd)),
            _ => None,
        },
        // `clear` (CSS2.1 §9.5.2 + Logical Properties). `none|left|right|both|
        // inline-start|inline-end`. NO hereda.
        "clear" => match value.trim().to_ascii_lowercase().as_str() {
            "none" => Some(DeclKind::Clear(Clear::None)),
            "left" => Some(DeclKind::Clear(Clear::Left)),
            "right" => Some(DeclKind::Clear(Clear::Right)),
            "both" => Some(DeclKind::Clear(Clear::Both)),
            "inline-start" => Some(DeclKind::Clear(Clear::InlineStart)),
            "inline-end" => Some(DeclKind::Clear(Clear::InlineEnd)),
            _ => None,
        },
        // `baseline-shift` (SVG / CSS Inline 3): `baseline | sub | super |
        // <length-percentage>`. NO hereda.
        "baseline-shift" => match value.trim().to_ascii_lowercase().as_str() {
            "baseline" => Some(DeclKind::BaselineShift(BaselineShift::Baseline)),
            "sub" => Some(DeclKind::BaselineShift(BaselineShift::Sub)),
            "super" => Some(DeclKind::BaselineShift(BaselineShift::Super)),
            _ => parse_length_or_pct(value).map(|l| DeclKind::BaselineShift(BaselineShift::Length(l))),
        },
        // Fase 7.519 — `float-defer` (CSS Page Floats 3). `none|last|<int>`.
        "float-defer" => {
            let v = value.trim().to_ascii_lowercase();
            match v.as_str() {
                "none" => Some(DeclKind::FloatDefer(FloatDefer::None)),
                "last" => Some(DeclKind::FloatDefer(FloatDefer::Last)),
                _ => v.parse::<i32>().ok().map(|n| DeclKind::FloatDefer(FloatDefer::By(n))),
            }
        }
        // Fase 7.520 — `float-reference` (CSS Page Floats 3).
        "float-reference" => match value.trim().to_ascii_lowercase().as_str() {
            "inline" => Some(DeclKind::FloatReference(FloatReference::Inline)),
            "column" => Some(DeclKind::FloatReference(FloatReference::Column)),
            "region" => Some(DeclKind::FloatReference(FloatReference::Region)),
            "page" => Some(DeclKind::FloatReference(FloatReference::Page)),
            _ => None,
        },
        // Fase 7.521 — `float-offset` (CSS Page Floats 3). `<length-percentage>`.
        "float-offset" => parse_length_px(value).map(DeclKind::FloatOffset),
        // Fase 7.522 — `box-decoration-break` (CSS Fragmentation 4).
        "box-decoration-break" | "-webkit-box-decoration-break" => match value
            .trim()
            .to_ascii_lowercase()
            .as_str()
        {
            "slice" => Some(DeclKind::BoxDecorationBreak(BoxDecorationBreak::Slice)),
            "clone" => Some(DeclKind::BoxDecorationBreak(BoxDecorationBreak::Clone)),
            _ => None,
        },
        // Fase 7.523 — `line-snap` (CSS Line Grid). HEREDA.
        "line-snap" => match value.trim().to_ascii_lowercase().as_str() {
            "none" => Some(DeclKind::LineSnap(LineSnap::None)),
            "baseline" => Some(DeclKind::LineSnap(LineSnap::Baseline)),
            "contain" => Some(DeclKind::LineSnap(LineSnap::Contain)),
            _ => None,
        },
        // Fase 7.524 — `line-grid` (CSS Line Grid). HEREDA.
        "line-grid" => match value.trim().to_ascii_lowercase().as_str() {
            "match" => Some(DeclKind::LineGrid(LineGrid::Match)),
            "create" => Some(DeclKind::LineGrid(LineGrid::Create)),
            _ => None,
        },
        // Fase 7.525 — `initial-letter` shorthand (CSS Inline 3). HEREDA.
        // Parse opaco hasta que un layout de drop-cap lo necesite.
        // Fase 7.747 — alias `-webkit-initial-letter` → estándar.
        "initial-letter" | "-webkit-initial-letter" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("normal") {
                Some(DeclKind::InitialLetter(None))
            } else {
                Some(DeclKind::InitialLetter(Some(v.to_string())))
            }
        }
        // Fase 7.526 — `highlight` (CSS Highlight API). HEREDA.
        "highlight" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("none") {
                Some(DeclKind::Highlight(None))
            } else {
                Some(DeclKind::Highlight(Some(v.to_string())))
            }
        }
        // Fase 7.527 — `ruby-merge` (CSS Ruby 1). HEREDA.
        "ruby-merge" => match value.trim().to_ascii_lowercase().as_str() {
            "separate" => Some(DeclKind::RubyMerge(RubyMerge::Separate)),
            "collapse" => Some(DeclKind::RubyMerge(RubyMerge::Collapse)),
            "auto" => Some(DeclKind::RubyMerge(RubyMerge::Auto)),
            _ => None,
        },
        // Fase 7.528 — `text-spacing` shorthand (CSS Text 4). HEREDA.
        // Parse opaco — `normal` reservado a None.
        "text-spacing" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("normal") {
                Some(DeclKind::TextSpacing(None))
            } else {
                Some(DeclKind::TextSpacing(Some(v.to_string())))
            }
        }
        // Fase 7.529 — `speak-as` (CSS Speech 1). HEREDA.
        "speak-as" => match value.trim().to_ascii_lowercase().as_str() {
            "normal" => Some(DeclKind::SpeakAs(SpeakAs::Normal)),
            "spell-out" => Some(DeclKind::SpeakAs(SpeakAs::SpellOut)),
            "digits" => Some(DeclKind::SpeakAs(SpeakAs::Digits)),
            "literal-punctuation" => Some(DeclKind::SpeakAs(SpeakAs::LiteralPunctuation)),
            "no-punctuation" => Some(DeclKind::SpeakAs(SpeakAs::NoPunctuation)),
            _ => None,
        },
        // Fase 7.530 — `voice-balance` (CSS Speech 1). -100..100. HEREDA.
        // Keywords `left|center|right|leftwards|rightwards` → -100/0/100/-50/50.
        "voice-balance" => match value.trim().to_ascii_lowercase().as_str() {
            "left" => Some(DeclKind::VoiceBalance(-100.0)),
            "leftwards" => Some(DeclKind::VoiceBalance(-50.0)),
            "center" => Some(DeclKind::VoiceBalance(0.0)),
            "rightwards" => Some(DeclKind::VoiceBalance(50.0)),
            "right" => Some(DeclKind::VoiceBalance(100.0)),
            other => other
                .parse::<f32>()
                .ok()
                .filter(|n| (-100.0..=100.0).contains(n))
                .map(DeclKind::VoiceBalance),
        },
        // Fase 7.531-7.533 — `voice-{pitch,rate,volume}` (CSS Speech 1).
        // Parse opaco — `medium`/`normal` reservados a None.
        "voice-pitch" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("medium") {
                Some(DeclKind::VoicePitch(None))
            } else {
                Some(DeclKind::VoicePitch(Some(v.to_string())))
            }
        }
        "voice-rate" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("normal") {
                Some(DeclKind::VoiceRate(None))
            } else {
                Some(DeclKind::VoiceRate(Some(v.to_string())))
            }
        }
        "voice-volume" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("medium") {
                Some(DeclKind::VoiceVolume(None))
            } else {
                Some(DeclKind::VoiceVolume(Some(v.to_string())))
            }
        }
        // Fase 7.534-7.537 — `pause-{before,after}` y `rest-{before,after}`
        // (CSS Speech 1). Parse opaco — `none` reservado a None.
        "pause-before" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("none") {
                Some(DeclKind::PauseBefore(None))
            } else {
                Some(DeclKind::PauseBefore(Some(v.to_string())))
            }
        }
        "pause-after" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("none") {
                Some(DeclKind::PauseAfter(None))
            } else {
                Some(DeclKind::PauseAfter(Some(v.to_string())))
            }
        }
        "rest-before" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("none") {
                Some(DeclKind::RestBefore(None))
            } else {
                Some(DeclKind::RestBefore(Some(v.to_string())))
            }
        }
        "rest-after" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("none") {
                Some(DeclKind::RestAfter(None))
            } else {
                Some(DeclKind::RestAfter(Some(v.to_string())))
            }
        }
        // Fase 7.538 — `cue-fade-duration` (CSS Speech 1). `<time>`.
        "cue-fade-duration" => parse_time_seconds(value.trim()).map(DeclKind::CueFadeDuration),
        // Fase 7.539-7.541 — `cue-{before,after}` y `cue` shorthand (CSS Speech 1).
        // Parse opaco — `none` reservado a None.
        "cue-before" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("none") {
                Some(DeclKind::CueBefore(None))
            } else {
                Some(DeclKind::CueBefore(Some(v.to_string())))
            }
        }
        "cue-after" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("none") {
                Some(DeclKind::CueAfter(None))
            } else {
                Some(DeclKind::CueAfter(Some(v.to_string())))
            }
        }
        "cue" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("none") {
                Some(DeclKind::Cue(None))
            } else {
                Some(DeclKind::Cue(Some(v.to_string())))
            }
        }
        // Fase 7.542 — `navigation-up` (CSS UI 3 legacy). Parse opaco —
        // `auto` reservado a None.
        "navigation-up" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("auto") {
                Some(DeclKind::NavigationUp(None))
            } else {
                Some(DeclKind::NavigationUp(Some(v.to_string())))
            }
        }
        // Fase 7.543 — `glyph-orientation-horizontal` (SVG 1.1 legacy).
        // `<angle>` en grados; sólo aceptamos 0/90/180/270 y los keywords
        // `0deg`/`90deg`/... — gramática extendida por simplicidad.
        "glyph-orientation-horizontal" => {
            let v = value.trim().to_ascii_lowercase();
            let num = v.strip_suffix("deg").unwrap_or(&v);
            num.parse::<f32>().ok().map(DeclKind::GlyphOrientationHorizontal)
        }
        // Fase 7.544-7.546 — `navigation-{down,left,right}` (CSS UI 3 legacy).
        // Parse opaco — `auto` reservado a None.
        "navigation-down" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("auto") {
                Some(DeclKind::NavigationDown(None))
            } else {
                Some(DeclKind::NavigationDown(Some(v.to_string())))
            }
        }
        "navigation-left" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("auto") {
                Some(DeclKind::NavigationLeft(None))
            } else {
                Some(DeclKind::NavigationLeft(Some(v.to_string())))
            }
        }
        "navigation-right" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("auto") {
                Some(DeclKind::NavigationRight(None))
            } else {
                Some(DeclKind::NavigationRight(Some(v.to_string())))
            }
        }
        // Fase 7.547 — `counter-increment-style` (CSS Lists 4). Parse opaco
        // — `decimal` reservado a None.
        "counter-increment-style" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("decimal") {
                Some(DeclKind::CounterIncrementStyle(None))
            } else {
                Some(DeclKind::CounterIncrementStyle(Some(v.to_string())))
            }
        }
        // Fase 7.548 — `overflow-clip-box` (CSS Overflow legacy).
        "overflow-clip-box" => match value.trim().to_ascii_lowercase().as_str() {
            "padding-box" => Some(DeclKind::OverflowClipBox(OverflowClipBox::PaddingBox)),
            "content-box" => Some(DeclKind::OverflowClipBox(OverflowClipBox::ContentBox)),
            _ => None,
        },
        // Fase 7.549-7.552 — familia `mask-border-*` (CSS Masking 1). Parse
        // opaco; el sentinel reservado va a `None`.
        // Fase 7.609-7.613 — `-webkit-mask-box-image-*` son los alias vendor
        // (de facto) de `mask-border-*`: enrutan al mismo handler/almacén.
        "mask-border-source" | "-webkit-mask-box-image-source" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("none") {
                Some(DeclKind::MaskBorderSource(None))
            } else {
                Some(DeclKind::MaskBorderSource(Some(v.to_string())))
            }
        }
        "mask-border-slice" | "-webkit-mask-box-image-slice" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v == "0" {
                Some(DeclKind::MaskBorderSlice(None))
            } else {
                Some(DeclKind::MaskBorderSlice(Some(v.to_string())))
            }
        }
        "mask-border-width" | "-webkit-mask-box-image-width" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("auto") {
                Some(DeclKind::MaskBorderWidth(None))
            } else {
                Some(DeclKind::MaskBorderWidth(Some(v.to_string())))
            }
        }
        "mask-border-outset" | "-webkit-mask-box-image-outset" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v == "0" {
                Some(DeclKind::MaskBorderOutset(None))
            } else {
                Some(DeclKind::MaskBorderOutset(Some(v.to_string())))
            }
        }
        // Fase 7.553 — `mask-border-repeat` (CSS Masking 1); Fase 7.613 alias.
        "mask-border-repeat" | "-webkit-mask-box-image-repeat" => match value.trim().to_ascii_lowercase().as_str() {
            "stretch" => Some(DeclKind::MaskBorderRepeat(MaskBorderRepeat::Stretch)),
            "repeat" => Some(DeclKind::MaskBorderRepeat(MaskBorderRepeat::Repeat)),
            "round" => Some(DeclKind::MaskBorderRepeat(MaskBorderRepeat::Round)),
            "space" => Some(DeclKind::MaskBorderRepeat(MaskBorderRepeat::Space)),
            _ => None,
        },
        // Fase 7.554 — `mask-border-mode` (CSS Masking 1).
        "mask-border-mode" => match value.trim().to_ascii_lowercase().as_str() {
            "luminance" => Some(DeclKind::MaskBorderMode(MaskBorderMode::Luminance)),
            "alpha" => Some(DeclKind::MaskBorderMode(MaskBorderMode::Alpha)),
            _ => None,
        },
        // Fase 7.555 — `caret-animation` (CSS UI 4).
        "caret-animation" => match value.trim().to_ascii_lowercase().as_str() {
            "auto" => Some(DeclKind::CaretAnimation(CaretAnimation::Auto)),
            "manual" => Some(DeclKind::CaretAnimation(CaretAnimation::Manual)),
            _ => None,
        },
        // Fase 7.556 — `scroll-marker-group` (CSS Overflow 5).
        "scroll-marker-group" => match value.trim().to_ascii_lowercase().as_str() {
            "none" => Some(DeclKind::ScrollMarkerGroup(ScrollMarkerGroup::None)),
            "before" => Some(DeclKind::ScrollMarkerGroup(ScrollMarkerGroup::Before)),
            "after" => Some(DeclKind::ScrollMarkerGroup(ScrollMarkerGroup::After)),
            _ => None,
        },
        // Fase 7.557 — `scroll-initial-target` (CSS Overflow 5).
        "scroll-initial-target" => match value.trim().to_ascii_lowercase().as_str() {
            "none" => Some(DeclKind::ScrollInitialTarget(ScrollInitialTarget::None)),
            "nearest" => Some(DeclKind::ScrollInitialTarget(ScrollInitialTarget::Nearest)),
            _ => None,
        },
        // Fase 7.558 — `corner-shape` (CSS Borders 4). Parse opaco —
        // `round` reservado a None.
        "corner-shape" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("round") {
                Some(DeclKind::CornerShape(None))
            } else {
                Some(DeclKind::CornerShape(Some(v.to_string())))
            }
        }
        // Fase 7.559 — `hyphenate-limit-lines` (CSS Text 4). `no-limit` → None.
        "hyphenate-limit-lines" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("no-limit") {
                Some(DeclKind::HyphenateLimitLines(None))
            } else {
                v.parse::<u32>().ok().map(|n| DeclKind::HyphenateLimitLines(Some(n)))
            }
        }
        // Fase 7.560 — `hyphenate-limit-last` (CSS Text 4).
        "hyphenate-limit-last" => match value.trim().to_ascii_lowercase().as_str() {
            "none" => Some(DeclKind::HyphenateLimitLast(HyphenateLimitLast::None)),
            "always" => Some(DeclKind::HyphenateLimitLast(HyphenateLimitLast::Always)),
            "column" => Some(DeclKind::HyphenateLimitLast(HyphenateLimitLast::Column)),
            "page" => Some(DeclKind::HyphenateLimitLast(HyphenateLimitLast::Page)),
            "spread" => Some(DeclKind::HyphenateLimitLast(HyphenateLimitLast::Spread)),
            _ => None,
        },
        // Fase 7.561 — `hyphenate-limit-zone` (CSS Text 4). `0` → None.
        "hyphenate-limit-zone" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v == "0" {
                Some(DeclKind::HyphenateLimitZone(None))
            } else {
                Some(DeclKind::HyphenateLimitZone(Some(v.to_string())))
            }
        }
        // Fase 7.562 — `interest-target` (interest invokers). Parse opaco.
        "interest-target" => {
            let v = value.trim();
            if v.is_empty() || v.eq_ignore_ascii_case("none") {
                Some(DeclKind::InterestTarget(None))
            } else {
                Some(DeclKind::InterestTarget(Some(v.to_string())))
            }
        }
        // Fase 7.563 — `interest-delay-start`. `normal` → None.
        "interest-delay-start" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("normal") {
                Some(DeclKind::InterestDelayStart(None))
            } else {
                Some(DeclKind::InterestDelayStart(Some(v.to_string())))
            }
        }
        // Fase 7.564 — `interest-delay-end`. `normal` → None.
        "interest-delay-end" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("normal") {
                Some(DeclKind::InterestDelayEnd(None))
            } else {
                Some(DeclKind::InterestDelayEnd(Some(v.to_string())))
            }
        }
        // Fase 7.565 — `azimuth` (CSS 2.1 aural). Parse opaco; `center` → None.
        "azimuth" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("center") {
                Some(DeclKind::Azimuth(None))
            } else {
                Some(DeclKind::Azimuth(Some(v.to_string())))
            }
        }
        _ => None,
    }
}
