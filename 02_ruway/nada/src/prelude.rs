//! Re-exports de los imports externos para los módulos hijos.
#![allow(unused_imports)]
pub(crate) use std::env;
pub(crate) use std::ffi::OsStr;
pub(crate) use std::fs;
pub(crate) use std::path::{Path, PathBuf};

pub(crate) use llimphi_theme::Theme;
pub(crate) use llimphi_ui::llimphi_layout::taffy::{
    prelude::{length, percent, FlexDirection, Rect, Size, Style},
    AlignItems, JustifyContent,
};
pub(crate) use llimphi_ui::llimphi_raster::peniko::Color;
pub(crate) use llimphi_ui::llimphi_text::Alignment;
pub(crate) use llimphi_ui::{App, Handle, Key, KeyEvent, KeyState, Modifiers, NamedKey, View, WheelDelta};
pub(crate) use llimphi_module_command_palette::{
    self as palette, Command as PaletteCommand, PaletteAction, PaletteMsg, PalettePalette,
    PaletteState,
};
pub(crate) use llimphi_module_diff_viewer::{
    self as diff, DiffAction, DiffMsg, DiffPalette, DiffState,
};
pub(crate) use llimphi_module_fif::{self as fif, FifAction, FifMsg, FifPalette, FifState};
pub(crate) use llimphi_module_file_picker::{
    self as picker, PickerAction, PickerMsg, PickerPalette, PickerState,
};
pub(crate) use llimphi_module_bookmarks::{
    self as bookmarks, BookmarksAction, BookmarksMsg, BookmarksOverlay, BookmarksPalette, BookmarksState,
};
pub(crate) use llimphi_module_mini_map::{
    self as minimap, MiniMapAction, MiniMapMsg, MiniMapPalette, MiniMapState, Snapshot as MiniMapSnapshot,
};
pub(crate) use llimphi_module_shuma_term::{
    self as term, ShumaTermAction, ShumaTermMsg, ShumaTermPalette, ShumaTermState,
};
pub(crate) use llimphi_module_symbol_outline::{
    self as outline, OutlineAction, OutlineMsg, OutlinePalette, OutlineState, SymbolItem,
};
pub(crate) use llimphi_widget_tabs::{tabs_view, TabsPalette, TabsSpec};
pub(crate) use llimphi_widget_text_editor::{
    all_matches, find_next, find_prev, text_editor_view_full, Clipboard, Diagnostic,
    EditorMetrics, EditorPalette, EditorState, FindState, Language, PointerEvent, Pos,
};
pub(crate) use llimphi_widget_text_editor_lsp::{
    CompletionItem, DefinitionLocation, DocumentSymbolEntry, HoverInfo, LspClient, NoopLspClient,
    RustAnalyzerClient, SignatureHelpInfo, TextEdit,
};
pub(crate) use llimphi_widget_text_input::{text_input_view, TextInputPalette, TextInputState};
pub(crate) use llimphi_widget_tree::{tree_view, TreePalette, TreeRow, TreeSpec};
