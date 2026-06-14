//! Brazos del dispatch `decl_kind_from_pair` — grupo dispatch_d.
//! Extraído de la mega-función original; se mantiene el orden exacto de
//! los brazos (props únicas) para preservar el comportamiento.
use super::super::*;
use super::*;

pub(crate) fn dispatch_d(p: &str, value: &str) -> Option<DeclKind> {
    match p {
        // Fase 7.669 — `-webkit-min-logical-width` (min-inline-size). `auto` → None.
        "-webkit-min-logical-width" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("auto") {
                Some(DeclKind::WebkitMinLogicalWidth(None))
            } else {
                Some(DeclKind::WebkitMinLogicalWidth(Some(v.to_string())))
            }
        }
        // Fase 7.670 — `-webkit-max-logical-width` (max-inline-size). `none` → None.
        "-webkit-max-logical-width" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("none") {
                Some(DeclKind::WebkitMaxLogicalWidth(None))
            } else {
                Some(DeclKind::WebkitMaxLogicalWidth(Some(v.to_string())))
            }
        }
        // Fase 7.671 — `-webkit-min-logical-height` (min-block-size). `auto` → None.
        "-webkit-min-logical-height" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("auto") {
                Some(DeclKind::WebkitMinLogicalHeight(None))
            } else {
                Some(DeclKind::WebkitMinLogicalHeight(Some(v.to_string())))
            }
        }
        // Fase 7.672 — `-webkit-max-logical-height` (max-block-size). `none` → None.
        "-webkit-max-logical-height" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("none") {
                Some(DeclKind::WebkitMaxLogicalHeight(None))
            } else {
                Some(DeclKind::WebkitMaxLogicalHeight(Some(v.to_string())))
            }
        }
        // Fase 7.673 — `-webkit-background-composite`. `source-over` → None.
        "-webkit-background-composite" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("source-over") {
                Some(DeclKind::WebkitBackgroundComposite(None))
            } else {
                Some(DeclKind::WebkitBackgroundComposite(Some(v.to_string())))
            }
        }
        // Fase 7.674 — `-webkit-border-before` (border-block-start). `none` → None.
        "-webkit-border-before" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("none") {
                Some(DeclKind::WebkitBorderBefore(None))
            } else {
                Some(DeclKind::WebkitBorderBefore(Some(v.to_string())))
            }
        }
        // Fase 7.675 — `-webkit-border-after` (border-block-end). `none` → None.
        "-webkit-border-after" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("none") {
                Some(DeclKind::WebkitBorderAfter(None))
            } else {
                Some(DeclKind::WebkitBorderAfter(Some(v.to_string())))
            }
        }
        // Fase 7.676 — `-webkit-border-start` (border-inline-start). `none` → None.
        "-webkit-border-start" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("none") {
                Some(DeclKind::WebkitBorderStart(None))
            } else {
                Some(DeclKind::WebkitBorderStart(Some(v.to_string())))
            }
        }
        // Fase 7.677 — `-webkit-border-end` (border-inline-end). `none` → None.
        "-webkit-border-end" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("none") {
                Some(DeclKind::WebkitBorderEnd(None))
            } else {
                Some(DeclKind::WebkitBorderEnd(Some(v.to_string())))
            }
        }
        // Fase 7.678 — `-webkit-border-horizontal-spacing`. `0` → None.
        "-webkit-border-horizontal-spacing" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v == "0" {
                Some(DeclKind::WebkitBorderHorizontalSpacing(None))
            } else {
                Some(DeclKind::WebkitBorderHorizontalSpacing(Some(v.to_string())))
            }
        }
        // Fase 7.679 — `-webkit-flow-into` (CSS Regions). `none` → None.
        "-webkit-flow-into" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("none") {
                Some(DeclKind::WebkitFlowInto(None))
            } else {
                Some(DeclKind::WebkitFlowInto(Some(v.to_string())))
            }
        }
        // Fase 7.680 — `-webkit-flow-from` (CSS Regions). `none` → None.
        "-webkit-flow-from" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("none") {
                Some(DeclKind::WebkitFlowFrom(None))
            } else {
                Some(DeclKind::WebkitFlowFrom(Some(v.to_string())))
            }
        }
        // Fase 7.681 — `-webkit-region-break-before`. `auto` → None.
        "-webkit-region-break-before" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("auto") {
                Some(DeclKind::WebkitRegionBreakBefore(None))
            } else {
                Some(DeclKind::WebkitRegionBreakBefore(Some(v.to_string())))
            }
        }
        // Fase 7.682 — `-webkit-region-break-after`. `auto` → None.
        "-webkit-region-break-after" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("auto") {
                Some(DeclKind::WebkitRegionBreakAfter(None))
            } else {
                Some(DeclKind::WebkitRegionBreakAfter(Some(v.to_string())))
            }
        }
        // Fase 7.683 — `-webkit-region-break-inside`. `auto` → None.
        "-webkit-region-break-inside" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("auto") {
                Some(DeclKind::WebkitRegionBreakInside(None))
            } else {
                Some(DeclKind::WebkitRegionBreakInside(Some(v.to_string())))
            }
        }
        // Fase 7.698 — `-webkit-border-before-color`. `currentcolor` → None.
        "-webkit-border-before-color" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("currentcolor") {
                Some(DeclKind::WebkitBorderBeforeColor(None))
            } else {
                Some(DeclKind::WebkitBorderBeforeColor(Some(v.to_string())))
            }
        }
        // Fase 7.699 — `-webkit-border-before-style`. `none` → None.
        "-webkit-border-before-style" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("none") {
                Some(DeclKind::WebkitBorderBeforeStyle(None))
            } else {
                Some(DeclKind::WebkitBorderBeforeStyle(Some(v.to_string())))
            }
        }
        // Fase 7.700 — `-webkit-border-before-width`. `medium` → None.
        "-webkit-border-before-width" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("medium") {
                Some(DeclKind::WebkitBorderBeforeWidth(None))
            } else {
                Some(DeclKind::WebkitBorderBeforeWidth(Some(v.to_string())))
            }
        }
        // Fase 7.701 — `-webkit-border-after-color`. `currentcolor` → None.
        "-webkit-border-after-color" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("currentcolor") {
                Some(DeclKind::WebkitBorderAfterColor(None))
            } else {
                Some(DeclKind::WebkitBorderAfterColor(Some(v.to_string())))
            }
        }
        // Fase 7.702 — `-webkit-border-after-style`. `none` → None.
        "-webkit-border-after-style" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("none") {
                Some(DeclKind::WebkitBorderAfterStyle(None))
            } else {
                Some(DeclKind::WebkitBorderAfterStyle(Some(v.to_string())))
            }
        }
        // Fase 7.703 — `-webkit-border-after-width`. `medium` → None.
        "-webkit-border-after-width" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("medium") {
                Some(DeclKind::WebkitBorderAfterWidth(None))
            } else {
                Some(DeclKind::WebkitBorderAfterWidth(Some(v.to_string())))
            }
        }
        // Fase 7.704 — `-webkit-border-start-color`. `currentcolor` → None.
        "-webkit-border-start-color" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("currentcolor") {
                Some(DeclKind::WebkitBorderStartColor(None))
            } else {
                Some(DeclKind::WebkitBorderStartColor(Some(v.to_string())))
            }
        }
        // Fase 7.705 — `-webkit-border-start-style`. `none` → None.
        "-webkit-border-start-style" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("none") {
                Some(DeclKind::WebkitBorderStartStyle(None))
            } else {
                Some(DeclKind::WebkitBorderStartStyle(Some(v.to_string())))
            }
        }
        // Fase 7.706 — `-webkit-border-start-width`. `medium` → None.
        "-webkit-border-start-width" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("medium") {
                Some(DeclKind::WebkitBorderStartWidth(None))
            } else {
                Some(DeclKind::WebkitBorderStartWidth(Some(v.to_string())))
            }
        }
        // Fase 7.707 — `-webkit-border-end-color`. `currentcolor` → None.
        "-webkit-border-end-color" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("currentcolor") {
                Some(DeclKind::WebkitBorderEndColor(None))
            } else {
                Some(DeclKind::WebkitBorderEndColor(Some(v.to_string())))
            }
        }
        // Fase 7.708 — `-webkit-border-end-style`. `none` → None.
        "-webkit-border-end-style" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("none") {
                Some(DeclKind::WebkitBorderEndStyle(None))
            } else {
                Some(DeclKind::WebkitBorderEndStyle(Some(v.to_string())))
            }
        }
        // Fase 7.709 — `-webkit-border-end-width`. `medium` → None.
        "-webkit-border-end-width" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("medium") {
                Some(DeclKind::WebkitBorderEndWidth(None))
            } else {
                Some(DeclKind::WebkitBorderEndWidth(Some(v.to_string())))
            }
        }
        // Fase 7.730 — `-webkit-margin-top-collapse`. `collapse` → None.
        "-webkit-margin-top-collapse" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("collapse") {
                Some(DeclKind::WebkitMarginTopCollapse(None))
            } else {
                Some(DeclKind::WebkitMarginTopCollapse(Some(v.to_string())))
            }
        }
        // Fase 7.731 — `-webkit-margin-bottom-collapse`. `collapse` → None.
        "-webkit-margin-bottom-collapse" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("collapse") {
                Some(DeclKind::WebkitMarginBottomCollapse(None))
            } else {
                Some(DeclKind::WebkitMarginBottomCollapse(Some(v.to_string())))
            }
        }
        // Fase 7.732 — `-webkit-margin-collapse` (shorthand). `collapse` → None.
        "-webkit-margin-collapse" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("collapse") {
                Some(DeclKind::WebkitMarginCollapse(None))
            } else {
                Some(DeclKind::WebkitMarginCollapse(Some(v.to_string())))
            }
        }
        // Fase 7.733 — `-webkit-border-vertical-spacing`. `0` → None.
        "-webkit-border-vertical-spacing" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v == "0" {
                Some(DeclKind::WebkitBorderVerticalSpacing(None))
            } else {
                Some(DeclKind::WebkitBorderVerticalSpacing(Some(v.to_string())))
            }
        }
        // Fase 7.734 — `-webkit-mask-source-type`. `alpha` → None.
        "-webkit-mask-source-type" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("alpha") {
                Some(DeclKind::WebkitMaskSourceType(None))
            } else {
                Some(DeclKind::WebkitMaskSourceType(Some(v.to_string())))
            }
        }
        // Fase 7.750 — `-webkit-marquee-direction`. `auto` → None.
        "-webkit-marquee-direction" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("auto") {
                Some(DeclKind::WebkitMarqueeDirection(None))
            } else {
                Some(DeclKind::WebkitMarqueeDirection(Some(v.to_string())))
            }
        }
        // Fase 7.751 — `-webkit-marquee-increment`. `6px` → None.
        "-webkit-marquee-increment" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("6px") {
                Some(DeclKind::WebkitMarqueeIncrement(None))
            } else {
                Some(DeclKind::WebkitMarqueeIncrement(Some(v.to_string())))
            }
        }
        // Fase 7.752 — `-webkit-marquee-repetition`. `infinite` → None.
        "-webkit-marquee-repetition" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("infinite") {
                Some(DeclKind::WebkitMarqueeRepetition(None))
            } else {
                Some(DeclKind::WebkitMarqueeRepetition(Some(v.to_string())))
            }
        }
        // Fase 7.753 — `-webkit-marquee-speed`. `normal` → None.
        "-webkit-marquee-speed" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("normal") {
                Some(DeclKind::WebkitMarqueeSpeed(None))
            } else {
                Some(DeclKind::WebkitMarqueeSpeed(Some(v.to_string())))
            }
        }
        // Fase 7.754 — `-webkit-marquee-style`. `scroll` → None.
        "-webkit-marquee-style" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("scroll") {
                Some(DeclKind::WebkitMarqueeStyle(None))
            } else {
                Some(DeclKind::WebkitMarqueeStyle(Some(v.to_string())))
            }
        }
        // Fase 7.755 — `-webkit-overflow-scrolling`. `auto` → None.
        "-webkit-overflow-scrolling" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("auto") {
                Some(DeclKind::WebkitOverflowScrolling(None))
            } else {
                Some(DeclKind::WebkitOverflowScrolling(Some(v.to_string())))
            }
        }
        // Fase 7.756 — `-webkit-line-grid`. `none` → None.
        "-webkit-line-grid" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("none") {
                Some(DeclKind::WebkitLineGrid(None))
            } else {
                Some(DeclKind::WebkitLineGrid(Some(v.to_string())))
            }
        }
        // Fase 7.757 — `-webkit-cursor-visibility`. `auto` → None.
        "-webkit-cursor-visibility" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("auto") {
                Some(DeclKind::WebkitCursorVisibility(None))
            } else {
                Some(DeclKind::WebkitCursorVisibility(Some(v.to_string())))
            }
        }
        // Fase 7.758 — `-webkit-border-fit`. `border` → None.
        "-webkit-border-fit" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("border") {
                Some(DeclKind::WebkitBorderFit(None))
            } else {
                Some(DeclKind::WebkitBorderFit(Some(v.to_string())))
            }
        }
        // Fase 7.759 — `-webkit-color-correction`. `default` → None.
        "-webkit-color-correction" => {
            let v = value.trim();
            if v.is_empty() { None }
            else if v.eq_ignore_ascii_case("default") {
                Some(DeclKind::WebkitColorCorrection(None))
            } else {
                Some(DeclKind::WebkitColorCorrection(Some(v.to_string())))
            }
        }
        // `scroll-margin-block` (Fase 7.417), `scroll-margin-inline` (Fase
        // 7.420), `scroll-padding-block` (Fase 7.423), `scroll-padding-inline`
        // (Fase 7.426) shorthands: ver `parse_declarations`.
        // Fase 7.793 — `-ms-touch-action` alias vendor (IE10).
        "touch-action" | "-ms-touch-action" => parse_touch_action(value).map(DeclKind::TouchAction),
        "clip-path" | "-webkit-clip-path" => Some(DeclKind::ClipPath(parse_clip_path(value))),
        "mask-image" => Some(DeclKind::MaskImage(parse_mask_image(value))),
        // `mask` shorthand: hoy sólo el subset image (igual que mask-image).
        "mask" | "-webkit-mask" | "-webkit-mask-image" => {
            Some(DeclKind::MaskImage(parse_mask_image(value)))
        }
        "content-visibility" => {
            parse_content_visibility(value).map(DeclKind::ContentVisibility)
        }
        "contain" => parse_contain(value).map(DeclKind::Contain),
        // Fase 7.684-7.688 — la familia `-webkit-column-*` es el alias vendor
        // (de facto) de `column-*`: enruta al mismo parser/almacén.
        // Fase 7.768 — `-moz-column-count` alias vendor legacy.
        "column-count" | "-webkit-column-count" | "-moz-column-count" => {
            Some(DeclKind::ColumnCount(parse_column_count(value)))
        }
        // Fase 7.769 — `-moz-column-width` alias vendor legacy.
        "column-width" | "-webkit-column-width" | "-moz-column-width" => {
            parse_length_or_pct(value).map(DeclKind::ColumnWidth)
        }
        // Fase 7.777 — `-moz-column-rule-width` alias vendor legacy.
        "column-rule-width" | "-webkit-column-rule-width" | "-moz-column-rule-width" => {
            parse_length_px(value).map(DeclKind::ColumnRuleWidth)
        }
        // Fase 7.778 — `-moz-column-rule-color` alias vendor legacy.
        "column-rule-color" | "-webkit-column-rule-color" | "-moz-column-rule-color" => {
            if is_current_color(value) {
                Some(DeclKind::ColumnRuleColor(None))
            } else {
                parse_color(value).map(|c| DeclKind::ColumnRuleColor(Some(c)))
            }
        }
        // `column-rule-style` y `column-rule` van por `parse_declarations`.
        // Fase 7.920 — `row-rule-width` / `row-rule-color` (CSS Gap Decorations 1).
        // `row-rule-style`, `row-rule` y los shorthands `rule*` van por
        // `parse_declarations` (multi-decl).
        "row-rule-width" => parse_length_px(value).map(DeclKind::RowRuleWidth),
        "row-rule-color" => {
            if is_current_color(value) {
                Some(DeclKind::RowRuleColor(None))
            } else {
                parse_color(value).map(|c| DeclKind::RowRuleColor(Some(c)))
            }
        }
        // Fase 7.796 — `-moz-column-fill` alias vendor legacy.
        "column-fill" | "-moz-column-fill" => parse_column_fill(value).map(DeclKind::ColumnFill),
        // Fase 7.779 — `-moz-column-span` alias vendor legacy.
        "column-span" | "-webkit-column-span" | "-moz-column-span" => {
            parse_column_span(value).map(DeclKind::ColumnSpan)
        }
        // `page-break-inside` (legacy CSS 2.1) = `break-inside` (subset).
        "break-inside" | "page-break-inside" => {
            parse_break_inside(value).map(DeclKind::BreakInside)
        }
        "table-layout" => parse_table_layout(value).map(DeclKind::TableLayout),
        "border-collapse" => parse_border_collapse(value).map(DeclKind::BorderCollapse),
        "border-spacing" => parse_border_spacing(value).map(|(h, v)| DeclKind::BorderSpacing { h, v }),
        // Fase 7.640 — `-epub-caption-side` (EPUB) alias de `caption-side`.
        "caption-side" | "-epub-caption-side" => {
            parse_caption_side(value).map(DeclKind::CaptionSide)
        }
        "empty-cells" => parse_empty_cells(value).map(DeclKind::EmptyCells),
        // `break-before` / `break-after` (CSS Fragmentation 3) + alias
        // legacy `page-break-before` / `page-break-after` (CSS 2.1, subset
        // auto/avoid/always/left/right).
        "break-before" | "page-break-before" => {
            parse_break_between(value).map(DeclKind::BreakBefore)
        }
        "break-after" | "page-break-after" => {
            parse_break_between(value).map(DeclKind::BreakAfter)
        }
        "orphans" => parse_positive_int(value).map(DeclKind::Orphans),
        "widows" => parse_positive_int(value).map(DeclKind::Widows),
        "color-scheme" => parse_color_scheme(value).map(DeclKind::ColorScheme),
        "counter-set" => Some(DeclKind::CounterSet(parse_counter_list(value, 0))),
        "quotes" => Some(DeclKind::Quotes(parse_quotes(value))),
        // Fase 7.761 — alias `-webkit-text-underline-position` → estándar.
        "text-underline-position" | "-webkit-text-underline-position" => {
            parse_text_underline_position(value).map(DeclKind::TextUnderlinePosition)
        }
        "text-justify" => parse_text_justify(value).map(DeclKind::TextJustify),
        // `color-adjust` es alias legacy de `print-color-adjust`.
        // Fase 7.748 — alias `-webkit-print-color-adjust` → estándar.
        "print-color-adjust" | "color-adjust" | "-webkit-print-color-adjust" => {
            parse_print_color_adjust(value).map(DeclKind::PrintColorAdjust)
        }
        "forced-color-adjust" => {
            parse_forced_color_adjust(value).map(DeclKind::ForcedColorAdjust)
        }
        // `-webkit-line-clamp` (de facto estándar) y `line-clamp` (CSS Overflow 4).
        "line-clamp" | "-webkit-line-clamp" => Some(DeclKind::LineClamp(parse_line_clamp(value))),
        "font-variant-caps" => {
            parse_font_variant_caps(value).map(DeclKind::FontVariantCaps)
        }
        "font-variant-numeric" => {
            parse_font_variant_numeric(value).map(DeclKind::FontVariantNumeric)
        }
        "font-variant-ligatures" => {
            parse_font_variant_ligatures(value).map(DeclKind::FontVariantLigatures)
        }
        "font-variant-east-asian" => {
            parse_font_variant_east_asian(value).map(DeclKind::FontVariantEastAsian)
        }
        "font-variant-position" => {
            parse_font_variant_position(value).map(DeclKind::FontVariantPosition)
        }
        // Fase 7.634-7.636 — la familia `-webkit-text-emphasis-*` es el alias
        // vendor (de facto) de `text-emphasis-*`: mismo parser/almacén.
        // Fase 7.642-7.643 — los `-epub-text-emphasis-{style,color}` (EPUB) al
        // mismo destino que los estándar/webkit.
        "text-emphasis-style" | "-webkit-text-emphasis-style"
        | "-epub-text-emphasis-style" => {
            parse_text_emphasis_style(value).map(DeclKind::TextEmphasisStyle)
        }
        "text-emphasis-color" | "-webkit-text-emphasis-color"
        | "-epub-text-emphasis-color" => {
            if is_current_color(value) {
                Some(DeclKind::TextEmphasisColor(None))
            } else {
                parse_color(value).map(|c| DeclKind::TextEmphasisColor(Some(c)))
            }
        }
        "text-emphasis-position" | "-webkit-text-emphasis-position" => {
            parse_text_emphasis_position(value).map(DeclKind::TextEmphasisPosition)
        }
        // `text-emphasis` shorthand: ver `parse_declarations`.
        // Fase 7.749 — alias `-webkit-ruby-position` → estándar.
        "ruby-position" | "-webkit-ruby-position" => parse_ruby_position(value).map(DeclKind::RubyPosition),
        // Fase 7.662/7.772/7.797 — `-webkit-`/`-moz-`/`-ms-transform-origin` alias vendor del shorthand.
        "transform-origin" | "-webkit-transform-origin" | "-moz-transform-origin" | "-ms-transform-origin" => {
            parse_transform_origin(value).map(DeclKind::TransformOrigin)
        }
        // Fase 7.740 — alias `-webkit-transform-style` → estándar.
        // Fase 7.775 — `-moz-transform-style` alias vendor legacy.
        "transform-style" | "-webkit-transform-style" | "-moz-transform-style" => {
            parse_transform_style(value).map(DeclKind::TransformStyle)
        }
        // Fase 7.741 — `-webkit-perspective` / Fase 7.773 — `-moz-perspective`.
        "perspective" | "-webkit-perspective" | "-moz-perspective" => parse_perspective(value).map(DeclKind::Perspective),
        // Fase 7.663 — `-webkit-perspective-origin` / Fase 7.776 — `-moz-perspective-origin`.
        "perspective-origin" | "-webkit-perspective-origin" | "-moz-perspective-origin" => {
            parse_perspective_origin(value).map(DeclKind::PerspectiveOrigin)
        }
        // Fase 7.742 — `-webkit-backface-visibility` / Fase 7.774 — `-moz-backface-visibility`.
        "backface-visibility" | "-webkit-backface-visibility" | "-moz-backface-visibility" => {
            parse_backface_visibility(value).map(DeclKind::BackfaceVisibility)
        }
        "scrollbar-width" => {
            parse_scrollbar_width(value).map(DeclKind::ScrollbarWidth)
        }
        "scrollbar-color" => {
            parse_scrollbar_color(value).map(DeclKind::ScrollbarColor)
        }
        "scrollbar-gutter" => {
            parse_scrollbar_gutter(value).map(DeclKind::ScrollbarGutter)
        }
        "overflow-anchor" => {
            parse_overflow_anchor(value).map(DeclKind::OverflowAnchor)
        }
        "overflow-clip-margin" => {
            parse_overflow_clip_margin(value).map(DeclKind::OverflowClipMargin)
        }
        // Fase 7.762 — `-webkit-text-align-last` / Fase 7.799 — `-moz-text-align-last`.
        "text-align-last" | "-webkit-text-align-last" | "-moz-text-align-last" => {
            parse_text_align_last(value).map(DeclKind::TextAlignLast)
        }
        "text-wrap" => parse_text_wrap(value).map(DeclKind::TextWrap),
        // Fase 7.631 — `-webkit-line-break` alias vendor de `line-break`.
        "line-break" | "-webkit-line-break" => {
            parse_line_break(value).map(DeclKind::LineBreak)
        }
        "hanging-punctuation" => {
            parse_hanging_punctuation(value).map(DeclKind::HangingPunctuation)
        }
        "text-decoration-skip-ink" => {
            parse_text_decoration_skip_ink(value)
                .map(DeclKind::TextDecorationSkipInk)
        }
        "font-optical-sizing" => {
            parse_font_optical_sizing(value).map(DeclKind::FontOpticalSizing)
        }
        "font-synthesis-weight" => {
            parse_auto_or_none(value).map(DeclKind::FontSynthesisWeight)
        }
        "font-synthesis-style" => {
            parse_auto_or_none(value).map(DeclKind::FontSynthesisStyle)
        }
        "font-synthesis-small-caps" => {
            parse_auto_or_none(value).map(DeclKind::FontSynthesisSmallCaps)
        }
        // `font-synthesis` shorthand: ver `parse_declarations`.
        "font-size-adjust" => {
            parse_font_size_adjust(value).map(DeclKind::FontSizeAdjust)
        }
        "image-orientation" => {
            parse_image_orientation(value).map(DeclKind::ImageOrientation)
        }
        "animation-timeline" => {
            parse_timeline_ref(value).map(DeclKind::AnimationTimeline)
        }
        "scroll-timeline-name" => {
            parse_dashed_ident_or_none(value).map(DeclKind::ScrollTimelineName)
        }
        "scroll-timeline-axis" => {
            parse_timeline_axis(value).map(DeclKind::ScrollTimelineAxis)
        }
        "view-timeline-name" => {
            parse_dashed_ident_or_none(value).map(DeclKind::ViewTimelineName)
        }
        "view-timeline-axis" => {
            parse_timeline_axis(value).map(DeclKind::ViewTimelineAxis)
        }
        "white-space-collapse" => {
            parse_white_space_collapse(value).map(DeclKind::WhiteSpaceCollapse)
        }
        "text-wrap-mode" => {
            parse_text_wrap_mode(value).map(DeclKind::TextWrapMode)
        }
        "text-wrap-style" => {
            parse_text_wrap_style(value).map(DeclKind::TextWrapStyle)
        }
        "text-spacing-trim" => {
            parse_text_spacing_trim(value).map(DeclKind::TextSpacingTrim)
        }
        "text-box-trim" => {
            parse_text_box_trim(value).map(DeclKind::TextBoxTrim)
        }
        "math-style" => parse_math_style(value).map(DeclKind::MathStyle),
        "math-depth" => parse_math_depth(value).map(DeclKind::MathDepth),
        "math-shift" => parse_math_shift(value).map(DeclKind::MathShift),
        "field-sizing" => {
            parse_field_sizing(value).map(DeclKind::FieldSizing)
        }
        // Fase 7.905 — overlay (CSS Position 4) y dynamic-range-limit (CSS
        // Color HDR 1). Plumb opaco.
        "overlay" => match value.trim().to_ascii_lowercase().as_str() {
            "none" => Some(DeclKind::Overlay(Overlay::None)),
            "auto" => Some(DeclKind::Overlay(Overlay::Auto)),
            _ => None,
        },
        "dynamic-range-limit" => match value.trim().to_ascii_lowercase().as_str() {
            "standard" => Some(DeclKind::DynamicRangeLimit(DynamicRangeLimit::Standard)),
            "no-limit" => Some(DeclKind::DynamicRangeLimit(DynamicRangeLimit::NoLimit)),
            "high" => Some(DeclKind::DynamicRangeLimit(DynamicRangeLimit::High)),
            "constrained" => Some(DeclKind::DynamicRangeLimit(DynamicRangeLimit::Constrained)),
            "constrained-high" => {
                Some(DeclKind::DynamicRangeLimit(DynamicRangeLimit::ConstrainedHigh))
            }
            _ => None,
        },
        "text-box-edge" => {
            parse_text_box_edge(value).map(DeclKind::TextBoxEdge)
        }
        "anchor-name" => parse_ident_list_or_none(value).map(DeclKind::AnchorName),
        "position-anchor" => {
            parse_ident_or_auto(value).map(DeclKind::PositionAnchor)
        }
        "anchor-scope" => {
            parse_anchor_scope(value).map(DeclKind::AnchorScope)
        }
        "view-transition-name" => {
            parse_dashed_ident_or_none(value).map(DeclKind::ViewTransitionName)
        }
        "view-transition-class" => {
            parse_ident_list_or_none(value).map(DeclKind::ViewTransitionClass)
        }
        "font-palette" => parse_font_palette(value).map(DeclKind::FontPalette),
        "font-variant-alternates" => parse_font_variant_alternates(value)
            .map(DeclKind::FontVariantAlternates),
        "background-attachment" => {
            parse_background_attachment(value).map(DeclKind::BackgroundAttachment)
        }
        "caret-shape" => parse_caret_shape(value).map(DeclKind::CaretShape),
        "baseline-source" => {
            parse_baseline_source(value).map(DeclKind::BaselineSource)
        }
        "alignment-baseline" => {
            parse_alignment_baseline(value).map(DeclKind::AlignmentBaseline)
        }
        "dominant-baseline" => {
            parse_dominant_baseline(value).map(DeclKind::DominantBaseline)
        }
        "paint-order" => parse_paint_order(value).map(DeclKind::PaintOrder),
        "marker-side" => parse_marker_side(value).map(DeclKind::MarkerSide),
        "fill" => parse_svg_paint(value).map(DeclKind::Fill),
        "stroke" => parse_svg_paint(value).map(DeclKind::Stroke),
        "fill-opacity" => parse_svg_opacity(value).map(DeclKind::FillOpacity),
        "stroke-opacity" => {
            parse_svg_opacity(value).map(DeclKind::StrokeOpacity)
        }
        "stroke-width" => {
            parse_length_or_pct(value).map(DeclKind::StrokeWidth)
        }
        "stroke-linecap" => {
            parse_stroke_linecap(value).map(DeclKind::StrokeLinecap)
        }
        "stroke-linejoin" => {
            parse_stroke_linejoin(value).map(DeclKind::StrokeLinejoin)
        }
        "stroke-miterlimit" => {
            parse_stroke_miterlimit(value).map(DeclKind::StrokeMiterlimit)
        }
        "stroke-dasharray" => {
            parse_stroke_dasharray(value).map(DeclKind::StrokeDasharray)
        }
        "stroke-dashoffset" => {
            parse_length_or_pct(value).map(DeclKind::StrokeDashoffset)
        }
        "fill-rule" => parse_fill_rule(value).map(DeclKind::FillRule),
        "clip-rule" => parse_fill_rule(value).map(DeclKind::ClipRule),
        "color-interpolation" => {
            parse_color_interpolation(value).map(DeclKind::ColorInterpolation)
        }
        "shape-rendering" => {
            parse_shape_rendering(value).map(DeclKind::ShapeRendering)
        }
        "vector-effect" => {
            parse_vector_effect(value).map(DeclKind::VectorEffect)
        }
        // `d` (SVG 2 §6) como propiedad CSS: `none | path(<string>)`.
        // Plumb opaco (no parseamos el path-data). NO hereda.
        "d" => {
            let raw = value.trim();
            if raw.eq_ignore_ascii_case("none") {
                Some(DeclKind::D(None))
            } else if raw.to_ascii_lowercase().starts_with("path(") && raw.ends_with(')') {
                Some(DeclKind::D(Some(raw.to_string())))
            } else {
                None
            }
        }
        // CSS Grid 3 (masonry, draft). NO heredan.
        "masonry-auto-flow" => {
            parse_masonry_auto_flow(value).map(DeclKind::MasonryAutoFlow)
        }
        "justify-tracks" => {
            parse_justify_tracks(value).map(DeclKind::JustifyTracks)
        }
        "align-tracks" => parse_align_tracks(value).map(DeclKind::AlignTracks),
        // SVG 2 `<solidcolor>`. NO heredan.
        "solid-color" => parse_color(value).map(DeclKind::SolidColor),
        "solid-opacity" => parse_svg_opacity(value).map(DeclKind::SolidOpacity),
        // `page` (CSS Paged Media 3): `auto | <custom-ident>`. NO hereda.
        "page" => {
            let v = value.trim();
            if v.eq_ignore_ascii_case("auto") {
                Some(DeclKind::Page(None))
            } else if !v.is_empty() && !v.contains(char::is_whitespace) {
                Some(DeclKind::Page(Some(v.to_string())))
            } else {
                None
            }
        }
        "text-anchor" => parse_text_anchor(value).map(DeclKind::TextAnchor),
        "color-rendering" => {
            parse_color_rendering(value).map(DeclKind::ColorRendering)
        }
        "color-interpolation-filters" => parse_color_interpolation_filters(value)
            .map(DeclKind::ColorInterpolationFilters),
        "glyph-orientation-vertical" => parse_glyph_orientation_vertical(value)
            .map(DeclKind::GlyphOrientationVertical),
        "transform-box" => parse_transform_box(value).map(DeclKind::TransformBox),
        "marker-start" => {
            parse_marker_ref(value).map(DeclKind::MarkerStart)
        }
        "marker-mid" => parse_marker_ref(value).map(DeclKind::MarkerMid),
        "marker-end" => parse_marker_ref(value).map(DeclKind::MarkerEnd),
        "mask-type" => parse_mask_type(value).map(DeclKind::MaskType),
        "mask-mode" => parse_mask_mode(value).map(DeclKind::MaskMode),
        // Fase 7.693-7.697 — la familia `-webkit-mask-*` (longhands) es el
        // alias vendor (de facto) de `mask-*`: mismo parser/almacén.
        "mask-clip" | "-webkit-mask-clip" => parse_mask_clip(value).map(DeclKind::MaskClip),
        "mask-composite" => {
            parse_mask_composite(value).map(DeclKind::MaskComposite)
        }
        "mask-origin" | "-webkit-mask-origin" => {
            parse_mask_origin(value).map(DeclKind::MaskOrigin)
        }
        "mask-repeat" | "-webkit-mask-repeat" => {
            // Reusa `parse_background_repeat` (devuelve `DeclKind::BackgroundRepeat`);
            // extraemos el valor y lo reemitimos como `MaskRepeat`.
            match parse_background_repeat(value) {
                Some(DeclKind::BackgroundRepeat(r)) => {
                    Some(DeclKind::MaskRepeat(r))
                }
                _ => None,
            }
        }
        "mask-position" | "-webkit-mask-position" => match parse_background_position(value) {
            Some(DeclKind::BackgroundPosition(p)) => {
                Some(DeclKind::MaskPosition(p))
            }
            _ => None,
        },
        "mask-size" | "-webkit-mask-size" => match parse_background_size(value) {
            Some(DeclKind::BackgroundSize(sz)) => {
                Some(DeclKind::MaskSize(sz))
            }
            _ => None,
        },
        "container-name" => {
            parse_ident_list_or_none(value).map(DeclKind::ContainerName)
        }
        "container-type" => {
            parse_container_type(value).map(DeclKind::ContainerType)
        }
        "flood-color" => {
            parse_color_or_current(value).map(DeclKind::FloodColor)
        }
        "flood-opacity" => parse_svg_opacity(value).map(DeclKind::FloodOpacity),
        "lighting-color" => {
            parse_color_or_current(value).map(DeclKind::LightingColor)
        }
        "stop-color" => {
            parse_color_or_current(value).map(DeclKind::StopColor)
        }
        "stop-opacity" => parse_svg_opacity(value).map(DeclKind::StopOpacity),
        // `columns` shorthand: ver `parse_declarations`.
        // `place-items`, `place-content`, `place-self`: ver `parse_declarations`.
        // Fase 7.873 — `text-indent: <len> && hanging? && each-line?`. Sólo
        // modelamos la longitud; las flags `hanging`/`each-line` se aceptan y
        // se ignoran (el layout no las aplica todavía).
        "text-indent" => {
            let len_tok = value
                .split_whitespace()
                .find(|t| !matches!(t.to_ascii_lowercase().as_str(), "hanging" | "each-line"))
                .unwrap_or(value.trim());
            parse_px_or_math(len_tok).map(DeclKind::TextIndent)
        }
        // Fase 7.856 — `word-spacing: normal` (valor inicial) = 0px, igual
        // que `letter-spacing` abajo.
        "word-spacing" => {
            if value.trim().eq_ignore_ascii_case("normal") {
                Some(DeclKind::WordSpacing(0.0))
            } else {
                parse_px_or_math(value).map(DeclKind::WordSpacing)
            }
        }
        "letter-spacing" => {
            // `normal` = sin tracking extra (0px).
            if value.trim().eq_ignore_ascii_case("normal") {
                Some(DeclKind::LetterSpacing(0.0))
            } else {
                parse_px_or_math(value).map(DeclKind::LetterSpacing)
            }
        }
        "text-shadow" => parse_text_shadows(value).map(DeclKind::TextShadows),
        // Fase 7.722/7.766/7.794 — `-webkit-`/`-moz-`/`-ms-transform` alias vendor de `transform`.
        "transform" | "-webkit-transform" | "-moz-transform" | "-ms-transform" => {
            parse_transforms(value).map(DeclKind::Transforms)
        }
        // Fase 7.826-7.828 — props individuales de transform (CSS Transforms 2).
        // Se guardan aparte y se componen en `transforms` al cierre del compute.
        "translate" => parse_translate_prop(value).map(DeclKind::Translate),
        "rotate" => parse_rotate_prop(value).map(DeclKind::Rotate),
        "scale" => parse_scale_prop(value).map(DeclKind::Scale),
        "grid-template-columns" => {
            parse_grid_template(value).map(DeclKind::GridTemplateColumns)
        }
        "grid-template-rows" => parse_grid_template(value).map(DeclKind::GridTemplateRows),
        // Fase 7.723-7.724 — `-webkit-animation` / `-webkit-transition` alias
        // vendor de los shorthands `animation` / `transition`.
        // Fase 7.771 — `-moz-animation` alias vendor del shorthand `animation`.
        "animation" | "-webkit-animation" | "-moz-animation" => parse_animation(value),
        // Fase 7.767 — `-moz-transition` alias vendor del shorthand `transition`.
        "transition" | "-webkit-transition" | "-moz-transition" => parse_transition(value),
        // Fase 7.822-7.825 — longhands `transition-*` (faltaban; sólo el
        // shorthand `transition` los clasificaba). Editan el 1er binding de la
        // lista (modelo de binding único, ver `transition_first` en decl.rs).
        // Tomamos la 1ª de la lista separada por coma (`first_comma`); alias
        // vendor `-webkit-`/`-moz-`.
        "transition-property"
        | "-webkit-transition-property"
        | "-moz-transition-property" => {
            let v = first_comma(value.trim());
            if v.eq_ignore_ascii_case("none") {
                Some(DeclKind::TransitionPropertyFirst(None))
            } else if v.is_empty() {
                None
            } else {
                Some(DeclKind::TransitionPropertyFirst(Some(v.to_ascii_lowercase())))
            }
        }
        "transition-duration"
        | "-webkit-transition-duration"
        | "-moz-transition-duration" => {
            parse_time(first_comma(value.trim())).map(DeclKind::TransitionDurationFirst)
        }
        "transition-delay" | "-webkit-transition-delay" | "-moz-transition-delay" => {
            parse_time(first_comma(value.trim())).map(DeclKind::TransitionDelayFirst)
        }
        "transition-timing-function"
        | "-webkit-transition-timing-function"
        | "-moz-transition-timing-function" => {
            parse_easing(&first_comma(value.trim()).to_ascii_lowercase())
                .map(DeclKind::TransitionTimingFirst)
        }
        // `grid-gap` (legacy) = `gap`.
        "grid-gap" => parse_gap(value).map(|(r, c)| DeclKind::Gap { row: r, column: c }),
        "grid-row-gap" => parse_length_px(value).map(DeclKind::RowGap),
        "grid-column-gap" => parse_length_px(value).map(DeclKind::ColumnGap),
        // `border: 1px solid #ccc` — shorthand. Devolvemos un único
        // DeclKind sintético: en realidad ya hay 3 sub-decls que el
        // caller debe emitir, así que delegamos a una ruta especial vía
        // parse_declarations (ver más arriba). Acá no podemos producir
        // varios, así que ignoramos — la entrada se rellena en
        // parse_declarations cuando ve `border`.
        "border" => None,
        _ => None,
    }
}
