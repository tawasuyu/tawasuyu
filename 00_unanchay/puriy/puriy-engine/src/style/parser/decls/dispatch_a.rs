//! Brazos del dispatch `decl_kind_from_pair` — grupo dispatch_a.
//! Extraído de la mega-función original; se mantiene el orden exacto de
//! los brazos (props únicas) para preservar el comportamiento.
use super::*;

pub(crate) fn dispatch_a(p: &str, value: &str) -> Option<DeclKind> {
    match p {
        // `color: currentColor` = heredar el color (default), así que lo
        // dropeamos (None → el color heredado queda en pie).
        "color" if is_current_color(value) => None,
        "color" => parse_color(value).map(DeclKind::Color),
        // `background` (shorthand) se expande en `parse_declarations` antes
        // de llegar acá; sólo el longhand `background-color` toma color suelto.
        "background-color" if is_current_color(value) => {
            Some(DeclKind::CurrentColor(ColorTarget::Background))
        }
        "background-color" => parse_color(value).map(DeclKind::Background),
        "display" => parse_display(value).map(DeclKind::Display),
        "font-size" => parse_font_size(value),
        "font-weight" => parse_weight(value).map(DeclKind::FontWeight),
        "font-style" => parse_font_style(value).map(DeclKind::FontStyle),
        "font-family" => Some(DeclKind::FontFamily(value.trim().to_string())),
        "margin" => parse_sides(value).map(DeclKind::Margin),
        // Fase 7.853 — longhands de margin/padding aceptan funciones
        // matemáticas (`calc`/`min`/`max`/`clamp`), igual que el shorthand
        // `parse_sides` desde la Fase 7.847.
        "margin-top" => parse_length_px_or_calc(value).map(DeclKind::MarginTop),
        "margin-right" => parse_length_px_or_calc(value).map(DeclKind::MarginRight),
        "margin-bottom" => parse_length_px_or_calc(value).map(DeclKind::MarginBottom),
        "margin-left" => parse_length_px_or_calc(value).map(DeclKind::MarginLeft),
        "padding" => parse_sides(value).map(DeclKind::Padding),
        "padding-top" => parse_length_px_or_calc(value).map(DeclKind::PaddingTop),
        "padding-right" => parse_length_px_or_calc(value).map(DeclKind::PaddingRight),
        "padding-bottom" => parse_length_px_or_calc(value).map(DeclKind::PaddingBottom),
        "padding-left" => parse_length_px_or_calc(value).map(DeclKind::PaddingLeft),
        "width" => parse_length_or_pct(value).map(DeclKind::Width),
        "height" => parse_length_or_pct(value).map(DeclKind::Height),
        "max-width" => parse_max_size(value).map(DeclKind::MaxWidth),
        "text-align" => parse_text_align(value).map(DeclKind::TextAlign),
        // Fase 7.831 — `line-height: normal` (valor inicial, comunísimo) se
        // descartaba. `None` ya modela "normal" (font-dependent ~1.2) en el
        // ComputedStyle; lo reseteamos explícito para no heredar un número.
        "line-height" => {
            if value.trim().eq_ignore_ascii_case("normal") {
                Some(DeclKind::LineHeightNormal)
            } else {
                parse_line_height(value).map(DeclKind::LineHeight)
            }
        }
        "border-width" => parse_px_or_math(value).map(DeclKind::BorderWidth),
        "border-color" if is_current_color(value) => {
            Some(DeclKind::CurrentColor(ColorTarget::BorderAll))
        }
        "border-color" => parse_color(value).map(DeclKind::BorderColor),
        "border-style" => parse_border_style(value).map(DeclKind::BorderEnabled),
        // Fase 7.727 — `-webkit-border-radius` alias vendor de `border-radius`.
        // Fase 7.764 — `-moz-border-radius` alias vendor legacy.
        // Fase 7.877 — un valor único acepta calc() (el multivalor lo
        // intercepta `parse_declarations`).
        "border-radius" | "-webkit-border-radius" | "-moz-border-radius" => {
            parse_px_or_math(value).map(DeclKind::BorderRadius)
        }
        "z-index" => {
            // `auto` → 0; sino int (Fase 7.872: o `calc()` → entero). Negativos OK.
            let v = value.trim();
            if v.eq_ignore_ascii_case("auto") {
                Some(DeclKind::ZIndex(0))
            } else {
                v.parse::<i32>()
                    .ok()
                    .or_else(|| parse_number_or_calc(v).map(|n| n.round() as i32))
                    .map(DeclKind::ZIndex)
            }
        }
        "content" => Some(DeclKind::Content(parse_content_value(value))),
        "counter-reset" => Some(DeclKind::CounterReset(parse_counter_list(value, 0))),
        "counter-increment" => Some(DeclKind::CounterIncrement(parse_counter_list(value, 1))),
        // Fase 7.726 — `-webkit-box-shadow` alias vendor de `box-shadow`.
        // Fase 7.765 — `-moz-box-shadow` alias vendor legacy.
        "box-shadow" | "-webkit-box-shadow" | "-moz-box-shadow" => {
            Some(DeclKind::BoxShadows(parse_box_shadows(value)))
        }
        // `text-decoration` (shorthand) se expande en `parse_declarations`.
        "text-decoration-line" | "-webkit-text-decoration-line" => {
            parse_text_decoration(value).map(DeclKind::TextDecoration)
        }
        "text-decoration-color" if is_current_color(value) => {
            Some(DeclKind::TextDecorationColor(None))
        }
        "text-decoration-color" => parse_color(value).map(|c| DeclKind::TextDecorationColor(Some(c))),
        "text-decoration-style" => {
            parse_text_decoration_style(value).map(DeclKind::TextDecorationStyle)
        }
        // `auto`/`from-font` → None (grosor derivado); longitud → px.
        "text-decoration-thickness" => match value.trim().to_ascii_lowercase().as_str() {
            "auto" | "from-font" => Some(DeclKind::TextDecorationThickness(None)),
            _ => parse_length_px(value).map(|p| DeclKind::TextDecorationThickness(Some(p))),
        },
        "text-underline-offset" => match value.trim().to_ascii_lowercase().as_str() {
            "auto" => Some(DeclKind::TextUnderlineOffset(None)),
            _ => parse_length_px(value).map(|p| DeclKind::TextUnderlineOffset(Some(p))),
        },
        "list-style-type" => parse_list_style_type(value).map(DeclKind::ListStyleType),
        "list-style-position" => {
            parse_list_style_position(value).map(DeclKind::ListStylePosition)
        }
        "list-style-image" => Some(DeclKind::ListStyleImage(parse_list_style_image(value))),
        // `list-style` shorthand: ruteado por `parse_declarations` para
        // emitir varias longhands en orden libre. Acá NO se dispatcha.
        "list-style" => None,
        // Fase 7.710-7.711 — la familia `-webkit-flex-*` es el alias vendor
        // (de facto, era prefijado en la era Flexbox 2012) de `flex-*`.
        // Fase 7.798 — `-ms-flex-direction` (IE10, valores idénticos a estándar).
        "flex-direction" | "-webkit-flex-direction" | "-ms-flex-direction" => {
            parse_flex_direction(value).map(DeclKind::FlexDirection)
        }
        "flex-wrap" | "-webkit-flex-wrap" => parse_flex_wrap(value).map(DeclKind::FlexWrap),
        // Fase 7.801 — `-ms-flex-wrap` (IE10). Divergencia de valor: IE usaba
        // `none` donde el estándar usa `nowrap`; lo normalizamos antes de parsear.
        "-ms-flex-wrap" => {
            let v = value.trim();
            let norm = if v.eq_ignore_ascii_case("none") { "nowrap" } else { v };
            parse_flex_wrap(norm).map(DeclKind::FlexWrap)
        }
        // Fase 7.716-7.718 — alias vendor Flexbox 2012 de la familia de
        // alineación (-webkit-justify-content / -align-items / -align-content).
        "justify-content" | "-webkit-justify-content" => {
            parse_justify_content(value).map(DeclKind::JustifyContent)
        }
        "align-items" | "-webkit-align-items" => {
            parse_align_items(value).map(DeclKind::AlignItems)
        }
        "align-content" | "-webkit-align-content" => {
            parse_align_content(value).map(DeclKind::AlignContent)
        }
        "justify-items" => parse_justify_items(value).map(DeclKind::JustifyItems),
        "justify-self" => parse_justify_self(value).map(DeclKind::JustifySelf),
        "gap" => parse_gap(value).map(|(r, c)| DeclKind::Gap { row: r, column: c }),
        // Fase 7.853 — `row-gap`/`column-gap` aceptan `normal` (→0) y
        // funciones matemáticas, vía `parse_gap` (que ya hace ambas).
        "row-gap" => parse_gap(value).map(|(r, _)| DeclKind::RowGap(r)),
        // Fase 7.689 — `-webkit-column-gap` / Fase 7.770 — `-moz-column-gap` alias vendor de `column-gap`.
        "column-gap" | "-webkit-column-gap" | "-moz-column-gap" => {
            parse_gap(value).map(|(c, _)| DeclKind::ColumnGap(c))
        }
        // Fase 7.728 — `-webkit-box-sizing` / Fase 7.763 — `-moz-box-sizing` alias vendor de `box-sizing`.
        "box-sizing" | "-webkit-box-sizing" | "-moz-box-sizing" => {
            parse_box_sizing(value).map(DeclKind::BoxSizing)
        }
        "min-width" => parse_length_or_pct(value).map(DeclKind::MinWidth),
        "min-height" => parse_length_or_pct(value).map(DeclKind::MinHeight),
        "max-height" => parse_max_size(value).map(DeclKind::MaxHeight),
        // `aspect-ratio: auto` resetea; `W / H` o un número crudo fijan la
        // relación. La forma `auto W/H` (auto + ratio) toma sólo el ratio.
        "aspect-ratio" => {
            let v = value.trim();
            if v.eq_ignore_ascii_case("auto") {
                Some(DeclKind::AspectRatio(None))
            } else {
                // Fase 7.876 — `auto` puede ir como prefijo O sufijo del ratio
                // (`auto 16/9` o `4 / 3 auto`); en ambos casos sólo nos importa
                // el ratio (el `auto` permite encoger por contenido, que no
                // modelamos aparte).
                let v = v.strip_prefix("auto").map(str::trim).unwrap_or(v);
                let v = v.strip_suffix("auto").map(str::trim).unwrap_or(v);
                parse_aspect_ratio(v).map(|r| DeclKind::AspectRatio(Some(r)))
            }
        }
        // Tamaños lógicos → físicos (LTR + escritura horizontal): inline ↔
        // width, block ↔ height. Fase 7.194.
        "inline-size" => parse_length_or_pct(value).map(DeclKind::Width),
        "block-size" => parse_length_or_pct(value).map(DeclKind::Height),
        "min-inline-size" => parse_length_or_pct(value).map(DeclKind::MinWidth),
        "min-block-size" => parse_length_or_pct(value).map(DeclKind::MinHeight),
        "max-inline-size" => parse_max_size(value).map(DeclKind::MaxWidth),
        "max-block-size" => parse_max_size(value).map(DeclKind::MaxHeight),
        // Fase 7.834 — `overflow: <x> [<y>]` de dos valores. El modelo es de
        // campo único (no separamos ejes todavía), así que tomamos el 1er
        // token (eje x). `overflow-x`/`-y` directos caen igual acá.
        // Fase 7.846 — `overflow-inline`/`overflow-block` lógicos. Bajo el
        // modelo de campo único (sin ejes separados) y escritura horizontal
        // LTR caen al mismo `Overflow` que sus físicos x/y.
        "overflow" | "overflow-x" | "overflow-y" | "overflow-inline" | "overflow-block" => {
            let first = value.split_whitespace().next().unwrap_or(value);
            parse_overflow(first).map(DeclKind::Overflow)
        }
        "white-space" => parse_white_space(value).map(DeclKind::WhiteSpace),
        "text-transform" => parse_text_transform(value).map(DeclKind::TextTransform),
        // Fase 7.729 — `-webkit-opacity` alias vendor legacy de `opacity`.
        // Fase 7.790 — `-moz-opacity` alias vendor legacy (pre-opacity Gecko).
        "opacity" | "-webkit-opacity" | "-moz-opacity" => parse_opacity(value).map(DeclKind::Opacity),
        // Fase 7.719 — `-webkit-align-self` alias vendor de `align-self`.
        "align-self" | "-webkit-align-self" => {
            parse_align_self(value).map(DeclKind::AlignSelf)
        }
        // Fase 7.803 — `-ms-flex-positive` (IE10) → `flex-grow` (número idéntico).
        // Fase 7.872 — acepta `calc()` que resuelva a número.
        "flex-grow" | "-webkit-flex-grow" | "-ms-flex-positive" => {
            parse_number_or_calc(value).map(DeclKind::FlexGrow)
        }
        // Fase 7.804 — `-ms-flex-negative` (IE10) → `flex-shrink` (número idéntico).
        "flex-shrink" | "-webkit-flex-shrink" | "-ms-flex-negative" => {
            parse_number_or_calc(value).map(DeclKind::FlexShrink)
        }
        // Fase 7.805 — `-ms-flex-preferred-size` (IE10) → `flex-basis` (length idéntico).
        // Fase 7.872 — `flex-basis: content` → Auto (dimensiona por contenido).
        "flex-basis" | "-webkit-flex-basis" | "-ms-flex-preferred-size" => {
            if value.trim().eq_ignore_ascii_case("content") {
                Some(DeclKind::FlexBasis(LengthVal::Auto))
            } else {
                parse_length_or_pct(value).map(DeclKind::FlexBasis)
            }
        }
        // `flex` y `outline` son shorthands múltiples — se expanden en
        // `parse_declarations` antes de llegar acá.
        "flex" | "outline" => None,
        // Fase 7.873 — `outline-width` acepta thin/medium/thick (igual que
        // border-width) además de length/calc.
        "outline-width" => parse_border_width_token(value).map(DeclKind::OutlineWidth),
        // Fase 7.864 — `invert` (CSS UI; invierte los píxeles del fondo) no es
        // representable sin leer el framebuffer; lo aproximamos a `currentColor`
        // (un outline visible que sigue al color del texto).
        "outline-color" if is_current_color(value) || value.trim().eq_ignore_ascii_case("invert") => {
            Some(DeclKind::CurrentColor(ColorTarget::Outline))
        }
        "outline-color" => parse_color(value).map(DeclKind::OutlineColor),
        "outline-style" => parse_border_style(value).map(DeclKind::OutlineStyle),
        // Fase 7.877 — acepta calc().
        "outline-offset" => parse_px_or_math(value).map(DeclKind::OutlineOffset),
        "background-image" => parse_background_image(value),
        // Fase 7.811 — `-webkit-background-size` / `-moz-background-size` alias vendor legacy.
        // Fase 7.866 — los longhands aceptan listas por coma (multi-capa); el
        // modelo guarda sólo la capa 0 en el campo base, así que tomamos el 1er
        // segmento (las capas 2..N las setea el shorthand `background`).
        "background-size" | "-webkit-background-size" | "-moz-background-size" => {
            parse_background_size(first_comma(value))
        }
        "background-position" => parse_background_position(first_comma(value)),
        "background-repeat" => parse_background_repeat(first_comma(value)),
        "background-origin" => parse_background_origin(value),
        // `-webkit-background-clip: text` es el spelling dominante en la web
        // para texto con gradiente — lo tratamos igual que el sin-prefijo.
        "background-clip" | "-webkit-background-clip" => parse_background_clip(value),
        "position" => parse_position(value).map(DeclKind::Position),
        "top" => parse_length_or_pct_or_auto(value).map(DeclKind::InsetTop),
        "right" => parse_length_or_pct_or_auto(value).map(DeclKind::InsetRight),
        "bottom" => parse_length_or_pct_or_auto(value).map(DeclKind::InsetBottom),
        "left" => parse_length_or_pct_or_auto(value).map(DeclKind::InsetLeft),
        "vertical-align" => parse_vertical_align(value).map(DeclKind::VerticalAlign),
        "visibility" => parse_visibility(value).map(DeclKind::Visibility),
        "pointer-events" => parse_pointer_events(value).map(DeclKind::PointerEvents),
        "object-fit" => parse_object_fit(value).map(DeclKind::ObjectFit),
        "object-position" => match parse_background_position(value) {
            // Reusa el parser de background-position; sólo cambia el destino.
            Some(DeclKind::BackgroundPosition(p)) => Some(DeclKind::ObjectPosition(p)),
            _ => None,
        },
        // `caret-color: auto | currentColor | <color>`. `currentColor` queda
        // como `None` (sigue al color heredado en el chrome eventual).
        "caret-color" => Some(DeclKind::CaretColor(parse_caret_color(value))),
        // `accent-color: auto | <color>`. Sin `currentColor` por espec.
        "accent-color" => Some(DeclKind::AccentColor(parse_auto_or_color(value))),
        "cursor" => parse_cursor(value).map(DeclKind::Cursor),
        "text-overflow" => parse_text_overflow(value).map(DeclKind::TextOverflow),
        "scroll-behavior" => parse_scroll_behavior(value).map(DeclKind::ScrollBehavior),
        "tab-size" | "-moz-tab-size" => parse_tab_size(value).map(DeclKind::TabSize),
        // CSS UI 4 — `user-select` con sus prefijos legacy.
        "user-select" | "-webkit-user-select" | "-moz-user-select" | "-ms-user-select" => {
            parse_user_select(value).map(DeclKind::UserSelect)
        }
        // `word-wrap` es alias legacy IE; CSS Text 3 los unificó.
        "overflow-wrap" | "word-wrap" => {
            parse_overflow_wrap(value).map(DeclKind::OverflowWrap)
        }
        // Fase 7.639 — `-epub-word-break` (perfil EPUB) alias de `word-break`.
        "word-break" | "-epub-word-break" => {
            parse_word_break(value).map(DeclKind::WordBreak)
        }
        "hyphens" | "-webkit-hyphens" | "-moz-hyphens" | "-ms-hyphens" => {
            parse_hyphens(value).map(DeclKind::Hyphens)
        }
        "resize" => parse_resize(value).map(DeclKind::Resize),
        // Fase 7.629 — `-webkit-writing-mode` es el alias vendor (de facto)
        // de `writing-mode`: enruta al mismo parser/almacén.
        // Fase 7.637 — `-epub-writing-mode` (perfil EPUB) al mismo destino.
        "writing-mode" | "-webkit-writing-mode" | "-epub-writing-mode" => {
            parse_writing_mode(value).map(DeclKind::WritingMode)
        }
        "direction" => parse_direction(value).map(DeclKind::Direction),
        "unicode-bidi" => parse_unicode_bidi(value).map(DeclKind::UnicodeBidi),
        "font-stretch" => parse_font_stretch(value).map(DeclKind::FontStretch),
        "image-rendering" => parse_image_rendering(value).map(DeclKind::ImageRendering),
        "mix-blend-mode" => parse_blend_mode(value).map(DeclKind::MixBlendMode),
        "background-blend-mode" => {
            Some(DeclKind::BackgroundBlendMode(parse_blend_mode_list(value)))
        }
        "isolation" => parse_isolation(value).map(DeclKind::Isolation),
        "will-change" => Some(DeclKind::WillChange(parse_will_change(value))),
        // Aliases legacy: `-webkit-appearance` y `-moz-appearance`.
        "appearance" | "-webkit-appearance" | "-moz-appearance" => {
            parse_appearance(value).map(DeclKind::Appearance)
        }
        // Fase 7.745 — alias `-webkit-font-kerning` → estándar.
        "font-kerning" | "-webkit-font-kerning" => parse_font_kerning(value).map(DeclKind::FontKerning),
        // Fase 7.746 — alias `-webkit-font-feature-settings` → estándar.
        // Fase 7.781 — `-moz-font-feature-settings` alias vendor legacy.
        "font-feature-settings" | "-webkit-font-feature-settings" | "-moz-font-feature-settings" => {
            Some(DeclKind::FontFeatureSettings(parse_font_feature_settings(value)))
        }
        "font-variation-settings" => {
            Some(DeclKind::FontVariationSettings(parse_font_variation_settings(value)))
        }
        "font-language-override" => {
            Some(DeclKind::FontLanguageOverride(parse_font_language_override(value)))
        }
        "text-rendering" => parse_text_rendering(value).map(DeclKind::TextRendering),
        // Fase 7.725 — `-webkit-filter` alias vendor de `filter`.
        "filter" | "-webkit-filter" => Some(DeclKind::Filter(parse_filter_list(value))),
        "backdrop-filter" | "-webkit-backdrop-filter" => {
            Some(DeclKind::BackdropFilter(parse_filter_list(value)))
        }
        // Fase 7.630 — `-webkit-text-orientation` alias vendor de
        // `text-orientation`. Fase 7.638 — `-epub-text-orientation` (EPUB).
        "text-orientation" | "-webkit-text-orientation" | "-epub-text-orientation" => {
            parse_text_orientation(value).map(DeclKind::TextOrientation)
        }
        "overscroll-behavior-x" => {
            parse_overscroll_behavior(value).map(DeclKind::OverscrollBehaviorX)
        }
        "overscroll-behavior-y" => {
            parse_overscroll_behavior(value).map(DeclKind::OverscrollBehaviorY)
        }
        // Fase 7.413 — `overscroll-behavior-block`. En LTR horizontal el
        // eje `block` es el vertical → mapea al longhand `-y`.
        "overscroll-behavior-block" => {
            parse_overscroll_behavior(value).map(DeclKind::OverscrollBehaviorY)
        }
        // Fase 7.414 — `overscroll-behavior-inline`. En LTR horizontal el
        // eje `inline` es el horizontal → mapea al longhand `-x`.
        "overscroll-behavior-inline" => {
            parse_overscroll_behavior(value).map(DeclKind::OverscrollBehaviorX)
        }
        // `overscroll-behavior` shorthand: ver `parse_declarations`.
        "scroll-snap-type" => parse_scroll_snap_type(value).map(DeclKind::ScrollSnapType),
        // `scroll-snap-align` shorthand: ver `parse_declarations`.
        "scroll-snap-align-block" => {
            parse_scroll_snap_align(value).map(DeclKind::ScrollSnapAlignBlock)
        }
        "scroll-snap-align-inline" => {
            parse_scroll_snap_align(value).map(DeclKind::ScrollSnapAlignInline)
        }
        "scroll-snap-stop" => parse_scroll_snap_stop(value).map(DeclKind::ScrollSnapStop),
        "scroll-padding" => parse_sides_lp(value).map(DeclKind::ScrollPadding),
        "scroll-padding-top" => parse_length_or_pct(value).map(DeclKind::ScrollPaddingTop),
        "scroll-padding-right" => {
            parse_length_or_pct(value).map(DeclKind::ScrollPaddingRight)
        }
        "scroll-padding-bottom" => {
            parse_length_or_pct(value).map(DeclKind::ScrollPaddingBottom)
        }
        "scroll-padding-left" => parse_length_or_pct(value).map(DeclKind::ScrollPaddingLeft),
        "scroll-margin" => parse_sides(value).map(DeclKind::ScrollMargin),
        "scroll-margin-top" => parse_length_px(value).map(DeclKind::ScrollMarginTop),
        "scroll-margin-right" => parse_length_px(value).map(DeclKind::ScrollMarginRight),
        "scroll-margin-bottom" => parse_length_px(value).map(DeclKind::ScrollMarginBottom),
        "scroll-margin-left" => parse_length_px(value).map(DeclKind::ScrollMarginLeft),
        // Fase 7.415 — `scroll-margin-block-start` = top en LTR horizontal.
        "scroll-margin-block-start" => {
            parse_length_px(value).map(DeclKind::ScrollMarginTop)
        }
        // Fase 7.416 — `scroll-margin-block-end` = bottom en LTR horizontal.
        "scroll-margin-block-end" => {
            parse_length_px(value).map(DeclKind::ScrollMarginBottom)
        }
        // Fase 7.418 — `scroll-margin-inline-start` = left en LTR horizontal.
        "scroll-margin-inline-start" => {
            parse_length_px(value).map(DeclKind::ScrollMarginLeft)
        }
        // Fase 7.419 — `scroll-margin-inline-end` = right en LTR horizontal.
        "scroll-margin-inline-end" => {
            parse_length_px(value).map(DeclKind::ScrollMarginRight)
        }
        // Fase 7.421 — `scroll-padding-block-start` = top en LTR horizontal.
        "scroll-padding-block-start" => {
            parse_length_or_pct(value).map(DeclKind::ScrollPaddingTop)
        }
        // Fase 7.422 — `scroll-padding-block-end` = bottom en LTR horizontal.
        "scroll-padding-block-end" => {
            parse_length_or_pct(value).map(DeclKind::ScrollPaddingBottom)
        }
        // Fase 7.424 — `scroll-padding-inline-start` = left en LTR horizontal.
        "scroll-padding-inline-start" => {
            parse_length_or_pct(value).map(DeclKind::ScrollPaddingLeft)
        }
        // Fase 7.425 — `scroll-padding-inline-end` = right en LTR horizontal.
        "scroll-padding-inline-end" => {
            parse_length_or_pct(value).map(DeclKind::ScrollPaddingRight)
        }
        // Fase 7.427 — `offset-path` (CSS Motion Path 1). Plumb: guarda el
        // valor crudo (sin parsear `path(...)` / `ray(...)` / `<url>`).
        // `none` o vacío → `None`. NO hereda.
        "offset-path" => {
            let raw = value.trim();
            if raw.is_empty() || raw.eq_ignore_ascii_case("none") {
                Some(DeclKind::OffsetPath(None))
            } else {
                Some(DeclKind::OffsetPath(Some(raw.to_string())))
            }
        }
        // Fase 7.428 — `offset-distance` (CSS Motion Path 1). Acepta
        // length o porcentaje (no `auto`). NO hereda.
        "offset-distance" => {
            parse_length_or_pct(value).map(DeclKind::OffsetDistance)
        }
        // Fase 7.429 — `hyphenate-character` (CSS Text 4). `auto` o un
        // string entre comillas. HEREDA. Plumb (no rompemos palabras).
        // Fase 7.632 — `-webkit-hyphenate-character` alias vendor.
        "hyphenate-character" | "-webkit-hyphenate-character" => {
            Some(DeclKind::HyphenateCharacter(parse_hyphenate_character(value)))
        }
        // Fase 7.430 — `hyphenate-limit-chars`. `auto | <int>{1,3}`. HEREDA.
        "hyphenate-limit-chars" => {
            parse_hyphenate_limit_chars(value).map(DeclKind::HyphenateLimitChars)
        }
        // Fase 7.431 — `text-size-adjust` (CSS Text Inline 3). HEREDA. Plumb.
        // Fase 7.791 — `-moz-text-size-adjust` / Fase 7.792 — `-ms-text-size-adjust` alias vendor.
        "text-size-adjust" | "-webkit-text-size-adjust" | "-moz-text-size-adjust" | "-ms-text-size-adjust" => {
            parse_text_size_adjust(value).map(DeclKind::TextSizeAdjust)
        }
        // Fase 7.432 — `line-height-step` (CSS Text 4 draft). HEREDA. Plumb.
        "line-height-step" => {
            parse_length_px(value).map(DeclKind::LineHeightStep)
        }
        // Fase 7.433 — `font-variant-emoji` (CSS Fonts 4). HEREDA. Plumb.
        "font-variant-emoji" => {
            parse_font_variant_emoji(value).map(DeclKind::FontVariantEmoji)
        }
        // Fase 7.434 — `contain-intrinsic-width` (CSS Containment 3). NO hereda. Plumb.
        "contain-intrinsic-width" => {
            parse_contain_intrinsic_size(value).map(DeclKind::ContainIntrinsicWidth)
        }
        // Fase 7.435 — `contain-intrinsic-height` (CSS Containment 3). NO hereda. Plumb.
        "contain-intrinsic-height" => {
            parse_contain_intrinsic_size(value).map(DeclKind::ContainIntrinsicHeight)
        }
        // Fase 7.436 — `contain-intrinsic-block-size` = height en horizontal LTR.
        "contain-intrinsic-block-size" => {
            parse_contain_intrinsic_size(value).map(DeclKind::ContainIntrinsicHeight)
        }
        // Fase 7.437 — `contain-intrinsic-inline-size` = width en horizontal LTR.
        "contain-intrinsic-inline-size" => {
            parse_contain_intrinsic_size(value).map(DeclKind::ContainIntrinsicWidth)
        }
        // Fase 7.438 — `contain-intrinsic-size` shorthand: ver `parse_declarations`.
        // Fase 7.439 — `background-position-x` (CSS Backgrounds 3). NO hereda.
        "background-position-x" => {
            parse_background_position_x(value).map(DeclKind::BackgroundPositionX)
        }
        // Fase 7.440 — `background-position-y` (CSS Backgrounds 3). NO hereda.
        "background-position-y" => {
            parse_background_position_y(value).map(DeclKind::BackgroundPositionY)
        }
        // Fase 7.441 — `grid-auto-flow` (CSS Grid 1). NO hereda.
        "grid-auto-flow" => parse_grid_auto_flow(value).map(DeclKind::GridAutoFlow),
        // Fase 7.442 — `grid-auto-columns` (CSS Grid 1). NO hereda.
        "grid-auto-columns" => parse_grid_template(value).map(DeclKind::GridAutoColumns),
        // Fase 7.443 — `grid-auto-rows` (CSS Grid 1). NO hereda.
        "grid-auto-rows" => parse_grid_template(value).map(DeclKind::GridAutoRows),
        // Fase 7.444 — `shape-outside` (CSS Shapes 1). Parse opaco (igual que
        // offset-path) — guardamos el valor crudo. NO hereda.
        // Fase 7.659 — `-webkit-shape-outside` alias vendor de `shape-outside`.
        "shape-outside" | "-webkit-shape-outside" => {
            let raw = value.trim();
            if raw.is_empty() || raw.eq_ignore_ascii_case("none") {
                Some(DeclKind::ShapeOutside(None))
            } else {
                Some(DeclKind::ShapeOutside(Some(raw.to_string())))
            }
        }
        // Fase 7.445 — `shape-margin` (CSS Shapes 1). `<length-or-pct>`
        // no-negativo. NO hereda. Fase 7.660 — `-webkit-shape-margin` alias.
        "shape-margin" | "-webkit-shape-margin" => {
            let v = parse_length_or_pct(value)?;
            match v {
                LengthVal::Px(n) if n < 0.0 => None,
                LengthVal::Pct(n) if n < 0.0 => None,
                _ => Some(DeclKind::ShapeMargin(v)),
            }
        }
        // Fase 7.446 — `shape-image-threshold` (CSS Shapes 1). Alpha [0..1].
        // Acepta también porcentaje (50% → 0.5). NO hereda.
        // Fase 7.661 — `-webkit-shape-image-threshold` alias vendor.
        "shape-image-threshold" | "-webkit-shape-image-threshold" => {
            parse_alpha_value(value).map(DeclKind::ShapeImageThreshold)
        }
        // Fase 7.447 — `text-combine-upright` (CSS Writing Modes 3). NO hereda.
        // Fase 7.633 — `-webkit-text-combine` es el nombre legacy WebKit.
        // Fase 7.641 — `-epub-text-combine` (EPUB) al mismo destino.
        "text-combine-upright" | "-webkit-text-combine" | "-epub-text-combine" => {
            parse_text_combine_upright(value).map(DeclKind::TextCombineUpright)
        }
        // Fase 7.448 — `ruby-align` (CSS Ruby 1). HEREDA.
        "ruby-align" => parse_ruby_align(value).map(DeclKind::RubyAlign),
        // Fase 7.449 — `offset-rotate` (CSS Motion Path 1). NO hereda.
        "offset-rotate" => parse_offset_rotate(value).map(DeclKind::OffsetRotate),
        // Fase 7.450 — `offset-anchor` (CSS Motion Path 1). `auto` o
        // `<position>`. Reusa `parse_background_position`.
        "offset-anchor" => {
            let v = value.trim();
            if v.eq_ignore_ascii_case("auto") {
                Some(DeclKind::OffsetAnchor(None))
            } else {
                match parse_background_position(v) {
                    Some(DeclKind::BackgroundPosition(p)) => {
                        Some(DeclKind::OffsetAnchor(Some(p)))
                    }
                    _ => None,
                }
            }
        }
        // Fase 7.451 — `offset-position` (CSS Motion Path 1). `auto`/`normal`
        // o `<position>`.
        "offset-position" => {
            let v = value.trim();
            if v.eq_ignore_ascii_case("auto") || v.eq_ignore_ascii_case("normal") {
                Some(DeclKind::OffsetPosition(None))
            } else {
                match parse_background_position(v) {
                    Some(DeclKind::BackgroundPosition(p)) => {
                        Some(DeclKind::OffsetPosition(Some(p)))
                    }
                    _ => None,
                }
            }
        }
        // Fase 7.452 — `object-view-box` (CSS Images 5). Parse opaco.
        "object-view-box" => {
            let raw = value.trim();
            if raw.is_empty() || raw.eq_ignore_ascii_case("none") {
                Some(DeclKind::ObjectViewBox(None))
            } else {
                Some(DeclKind::ObjectViewBox(Some(raw.to_string())))
            }
        }
        // Fase 7.453 — `ruby-overhang` (CSS Ruby 1). HEREDA.
        "ruby-overhang" => match value.trim().to_ascii_lowercase().as_str() {
            "auto" => Some(DeclKind::RubyOverhang(RubyOverhang::Auto)),
            "none" => Some(DeclKind::RubyOverhang(RubyOverhang::None)),
            _ => None,
        },
        // Fase 7.454 — `block-step-size` (CSS Inline Layout 3). `none | <length>`.
        "block-step-size" => parse_block_step_size(value).map(DeclKind::BlockStepSize),
        // Fase 7.455 — `block-step-insert` (CSS Inline Layout 3).
        "block-step-insert" => match value.trim().to_ascii_lowercase().as_str() {
            "margin-box" => Some(DeclKind::BlockStepInsert(BlockStepInsert::MarginBox)),
            "padding-box" => Some(DeclKind::BlockStepInsert(BlockStepInsert::PaddingBox)),
            _ => None,
        },
        // Fase 7.456 — `block-step-align` (CSS Inline Layout 3).
        "block-step-align" => match value.trim().to_ascii_lowercase().as_str() {
            "auto" => Some(DeclKind::BlockStepAlign(BlockStepAlign::Auto)),
            "center" => Some(DeclKind::BlockStepAlign(BlockStepAlign::Center)),
            "start" => Some(DeclKind::BlockStepAlign(BlockStepAlign::Start)),
            "end" => Some(DeclKind::BlockStepAlign(BlockStepAlign::End)),
            _ => None,
        },
        // Fase 7.457 — `block-step-round` (CSS Inline Layout 3).
        "block-step-round" => match value.trim().to_ascii_lowercase().as_str() {
            "up" => Some(DeclKind::BlockStepRound(BlockStepRound::Up)),
            "down" => Some(DeclKind::BlockStepRound(BlockStepRound::Down)),
            "nearest" => Some(DeclKind::BlockStepRound(BlockStepRound::Nearest)),
            _ => None,
        },
        // Fase 7.458 — `block-step` shorthand: ver `parse_declarations`.
        // Fase 7.459 — `position-visibility` (CSS Anchor Positioning 1).
        "position-visibility" => match value.trim().to_ascii_lowercase().as_str() {
            "always" => {
                Some(DeclKind::PositionVisibility(PositionVisibility::Always))
            }
            "anchors-visible" => Some(DeclKind::PositionVisibility(
                PositionVisibility::AnchorsVisible,
            )),
            "no-overflow" => Some(DeclKind::PositionVisibility(
                PositionVisibility::NoOverflow,
            )),
            _ => None,
        },
        // Fase 7.460 — `position-try-order` (CSS Anchor Positioning 1).
        "position-try-order" => parse_position_try_order(value)
            .map(DeclKind::PositionTryOrder),
        // Fase 7.461 — `position-try-fallbacks` (CSS Anchor Positioning 1).
        // `none` o lista de `<dashed-ident>` separados por coma. Tokens
        // distintos a un dashed-ident (ej. `flip-block`) los aceptamos como
        // string opaco — el chrome no implementa try yet.
        "position-try-fallbacks" => {
            parse_position_try_fallbacks(value).map(DeclKind::PositionTryFallbacks)
        }
        // Fase 7.462 — `position-try` shorthand: ver `parse_declarations`.
        // Fase 7.463 — `position-area` (CSS Anchor Positioning 1). Parse opaco.
        "position-area" => {
            let raw = value.trim();
            if raw.is_empty() || raw.eq_ignore_ascii_case("none") {
                Some(DeclKind::PositionArea(None))
            } else {
                Some(DeclKind::PositionArea(Some(raw.to_string())))
            }
        }
        // Fase 7.464 — `animation-range-start` (CSS Animations 2).
        "animation-range-start" => {
            parse_animation_range(value).map(DeclKind::AnimationRangeStart)
        }
        // Fase 7.465 — `animation-range-end` (CSS Animations 2).
        "animation-range-end" => {
            parse_animation_range(value).map(DeclKind::AnimationRangeEnd)
        }
        // Fase 7.466 — `animation-range` shorthand: ver `parse_declarations`.
        // Fase 7.467 — `transition-behavior` (CSS Transitions 2).
        "transition-behavior" => match value.trim().to_ascii_lowercase().as_str() {
            "normal" => Some(DeclKind::TransitionBehavior(TransitionBehavior::Normal)),
            "allow-discrete" => Some(DeclKind::TransitionBehavior(
                TransitionBehavior::AllowDiscrete,
            )),
            _ => None,
        },
        // Fase 7.468 — `interpolate-size` (CSS Values 5). HEREDA.
        "interpolate-size" => match value.trim().to_ascii_lowercase().as_str() {
            "numeric-only" => {
                Some(DeclKind::InterpolateSize(InterpolateSize::NumericOnly))
            }
            "allow-keywords" => {
                Some(DeclKind::InterpolateSize(InterpolateSize::AllowKeywords))
            }
            _ => None,
        },
        // Fase 7.469 — `view-timeline-inset: <inset>{1,2}` (CSS Scroll-Driven
        // Animations 1). Cada inset acepta `auto | <length-percentage>`.
        // `auto` se trata como `0px` (plumb sin runtime de timeline). Con 1
        // valor se aplica a ambos lados.
        "view-timeline-inset" => {
            let parts: Vec<&str> = value.split_whitespace().collect();
            if parts.is_empty() || parts.len() > 2 {
                None
            } else {
                let parse = |p: &str| -> Option<LengthVal> {
                    if p.eq_ignore_ascii_case("auto") {
                        Some(LengthVal::Px(0.0))
                    } else {
                        parse_length_or_pct(p)
                    }
                };
                let a = parse(parts[0])?;
                let b = if parts.len() == 2 {
                    parse(parts[1])?
                } else {
                    a
                };
                Some(DeclKind::ViewTimelineInset(a, b))
            }
        }
        // Fase 7.470 — `font-synthesis-position` (CSS Fonts 4). HEREDA.
        "font-synthesis-position" => {
            parse_auto_or_none(value).map(DeclKind::FontSynthesisPosition)
        }
        // Fase 7.471 — `scroll-timeline` shorthand: ver `parse_declarations`.
        // Fase 7.472 — `view-timeline` shorthand: ver `parse_declarations`.
        // Fase 7.473 — `interactivity` (CSS UI 4). HEREDA.
        "interactivity" => match value.trim().to_ascii_lowercase().as_str() {
            "auto" => Some(DeclKind::Interactivity(Interactivity::Auto)),
            "inert" => Some(DeclKind::Interactivity(Interactivity::Inert)),
            _ => None,
        },
        // Fase 7.474-7.478 — geometría SVG (`cx`, `cy`, `r`, `rx`, `ry`).
        // SVG2 promueve estos atributos a propiedades CSS — `<length-percentage>`
        // para los 5; `auto` válido sólo en `rx`/`ry` (sentinel `LengthVal::Auto`).
        // NO heredan.
        "cx" => parse_length_or_pct(value).map(DeclKind::Cx),
        "cy" => parse_length_or_pct(value).map(DeclKind::Cy),
        "r" => parse_length_or_pct(value).map(DeclKind::R),
        "rx" => parse_length_or_pct(value).map(DeclKind::Rx),
        "ry" => parse_length_or_pct(value).map(DeclKind::Ry),
        // `x` / `y` (SVG 2): posición de `<rect>`/`<image>`/`<foreignObject>`
        // como props CSS. `<length-percentage>`. Default `Px(0)`. NO heredan.
        "x" => parse_length_or_pct(value).map(DeclKind::X),
        "y" => parse_length_or_pct(value).map(DeclKind::Y),
        // Fase 7.479 — `order` (CSS Flexbox/Grid). `<integer>`. Default 0.
        // Fase 7.715 — `-webkit-order` / Fase 7.802 — `-ms-flex-order` (IE10) alias de `order`.
        // Fase 7.872 — acepta `calc()` que resuelva a entero.
        "order" | "-webkit-order" | "-ms-flex-order" => {
            value
                .trim()
                .parse::<i32>()
                .ok()
                .or_else(|| parse_number_or_calc(value).map(|n| n.round() as i32))
                .map(DeclKind::Order)
        }
        // Fase 7.480 — `path-length` (SVG2). `none | <number>`. NO hereda.
        "path-length" => {
            let v = value.trim();
            if v.eq_ignore_ascii_case("none") {
                Some(DeclKind::PathLength(None))
            } else {
                v.parse::<f32>()
                    .ok()
                    .filter(|n| *n >= 0.0)
                    .map(|n| DeclKind::PathLength(Some(n)))
            }
        }
        // Fase 7.481 — `animation-composition` (CSS Animations 2). Es una
        // lista por coma (una entrada por animación); guardamos la 1ª, igual
        // que el resto de longhands de animation.
        "animation-composition" => match first_comma(value).trim().to_ascii_lowercase().as_str() {
            "replace" => Some(DeclKind::AnimationComposition(AnimationComposition::Replace)),
            "add" => Some(DeclKind::AnimationComposition(AnimationComposition::Add)),
            "accumulate" => Some(DeclKind::AnimationComposition(
                AnimationComposition::Accumulate,
            )),
            _ => None,
        },
        // Fase 7.482 — `timeline-scope` (CSS Scroll-Driven Animations).
        // `none | <dashed-ident>#`.
        "timeline-scope" => {
            let v = value.trim();
            if v.eq_ignore_ascii_case("none") || v.is_empty() {
                Some(DeclKind::TimelineScope(Vec::new()))
            } else {
                let names: Vec<String> = v
                    .split(',')
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect();
                if names.is_empty() {
                    None
                } else {
                    Some(DeclKind::TimelineScope(names))
                }
            }
        }
        // Fase 7.483 — `reading-order` (CSS Inline 3). `<integer>`. NO hereda.
        "reading-order" => value
            .trim()
            .parse::<i32>()
            .ok()
            .map(DeclKind::ReadingOrder),
        // Fase 7.484 — `reading-flow` (CSS Display 4). Enum focus-order.
        "reading-flow" => match value.trim().to_ascii_lowercase().as_str() {
            "normal" => Some(DeclKind::ReadingFlow(ReadingFlow::Normal)),
            "flex-visual" => Some(DeclKind::ReadingFlow(ReadingFlow::FlexVisual)),
            "flex-flow" => Some(DeclKind::ReadingFlow(ReadingFlow::FlexFlow)),
            "grid-rows" => Some(DeclKind::ReadingFlow(ReadingFlow::GridRows)),
            "grid-columns" => Some(DeclKind::ReadingFlow(ReadingFlow::GridColumns)),
            "grid-order" => Some(DeclKind::ReadingFlow(ReadingFlow::GridOrder)),
            _ => None,
        },
        // Fase 7.485 — `image-resolution` (CSS Images 4).
        // `[ from-image || <resolution> ] && snap?`. HEREDA.
        "image-resolution" => parse_image_resolution(value).map(DeclKind::ImageResolution),
        // Fase 7.486 — `bookmark-level` (CSS GCPM). `none | <integer>`.
        "bookmark-level" => {
            let v = value.trim();
            if v.eq_ignore_ascii_case("none") {
                Some(DeclKind::BookmarkLevel(None))
            } else {
                v.parse::<u32>()
                    .ok()
                    .filter(|n| *n >= 1)
                    .map(|n| DeclKind::BookmarkLevel(Some(n)))
            }
        }
        // Fase 7.487 — `bookmark-state` (CSS GCPM). `open | closed`.
        "bookmark-state" => match value.trim().to_ascii_lowercase().as_str() {
            "open" => Some(DeclKind::BookmarkState(BookmarkState::Open)),
            "closed" => Some(DeclKind::BookmarkState(BookmarkState::Closed)),
            _ => None,
        },
        // Fase 7.488 — `bookmark-label` (CSS GCPM). `none | <content-list>`.
        // Parse opaco: guardamos el value crudo para que un renderer GCPM
        // lo evalúe; `none` reservado.
        "bookmark-label" => {
            let v = value.trim();
            if v.is_empty() {
                None
            } else if v.eq_ignore_ascii_case("none") {
                Some(DeclKind::BookmarkLabel(None))
            } else {
                Some(DeclKind::BookmarkLabel(Some(v.to_string())))
            }
        }
        _ => None,
    }
}
