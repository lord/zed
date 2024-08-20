use editor::actions::{Backspace, NewlineAbove, NewlineBelow, Paste};
use editor::display_map::DisplaySnapshot;
use editor::scroll::Autoscroll;
use editor::Editor;
use gpui::{actions, impl_actions, AppContext, ClipboardEntry, ViewContext, WindowContext};
use gpui::{Action, KeyContext};
use language::{CursorShape, Point};
use multi_buffer::{MultiBufferRow, ToPoint};
use serde::Deserialize;
use std::cmp;
use std::iter::Iterator;
use std::ops::Range;
use text::Selection;

struct DanceTag;

#[derive(Clone, Deserialize, PartialEq)]
struct SwitchMode(String);

impl_actions!(dance, [SwitchMode,]);
actions!(dance, [SelectLine, PasteAbove, PasteBelow,]);

/// Initializes the `vim` crate.
pub fn init(cx: &mut AppContext) {
    cx.observe_new_views(|editor: &mut Editor, cx| register(editor, cx))
        .detach();
}

fn make_key_context(mode: String) -> KeyContext {
    let mut key_context = KeyContext::new_with_defaults();
    key_context.set("dance_mode", mode.to_string());
    key_context
}

fn select_line(editor: &mut Editor, _: &SelectLine, cx: &mut ViewContext<Editor>) {
    let display_map = editor.display_map.update(cx, |map, cx| map.snapshot(cx));
    let mut selections = editor.selections.all::<Point>(cx);
    let max_point = display_map.buffer_snapshot.max_point();
    for selection in &mut selections {
        let rows = {
            let start = selection.start.to_point(&display_map.buffer_snapshot);
            let mut end = selection.end.to_point(&display_map.buffer_snapshot);
            if start.row != end.row && end.column == 0 {
                end.row -= 1;
            }

            let buffer_start = display_map.prev_line_boundary(start).0;
            let buffer_end = display_map.next_line_boundary(end).0;
            MultiBufferRow(buffer_start.row)..MultiBufferRow(buffer_end.row + 1)
        };
        selection.start = Point::new(rows.start.0, 0);
        selection.end = std::cmp::min(max_point, Point::new(rows.end.0, 0));
        selection.reversed = true;
    }

    editor.change_selections(Some(Autoscroll::fit()), cx, |s| {
        s.select(selections);
    });
}

fn clipboard_ends_in_newline(cx: &mut ViewContext<Editor>) -> bool {
    if let Some(item) = cx.read_from_clipboard() {
        item.entries().len() > 0
            && item.entries().iter().all(|entry| match entry {
                ClipboardEntry::Image(_) => false,
                ClipboardEntry::String(text) => {
                    let chars = text.text();
                    chars.chars().last() == Some('\n')
                }
            })
    } else {
        false
    }
}

fn paste_above(editor: &mut Editor, _: &PasteAbove, cx: &mut ViewContext<Editor>) {
    let ends_in_newline = clipboard_ends_in_newline(cx);
    if ends_in_newline {
        editor.newline_above(&NewlineAbove, cx);
    }
    editor.paste(&Paste, cx);
    if ends_in_newline {
        editor.backspace(&Backspace, cx);
    }
}

fn paste_below(editor: &mut Editor, _: &PasteBelow, cx: &mut ViewContext<Editor>) {
    let ends_in_newline = clipboard_ends_in_newline(cx);
    if ends_in_newline {
        editor.newline_below(&NewlineBelow, cx);
    }
    editor.paste(&Paste, cx);
    if ends_in_newline {
        editor.backspace(&Backspace, cx);
    }
}

fn switch_mode(
    editor: &mut Editor,
    &SwitchMode(ref mode): &SwitchMode,
    cx: &mut ViewContext<Editor>,
) {
    editor.set_keymap_context_layer::<DanceTag>(make_key_context(mode.to_string()), cx);
    if mode == "default" {
        editor.set_cursor_shape(CursorShape::Bar, cx);
    } else {
        editor.set_cursor_shape(CursorShape::WideBar, cx);
    }
}

fn register_editor_action<T: Action>(
    editor: &mut Editor,
    cx: &mut ViewContext<Editor>,
    f: fn(&mut Editor, &T, &mut ViewContext<Editor>),
) {
    let editor_handle = cx.view().downgrade();
    editor
        .register_action::<T>(move |mode, cx: &mut WindowContext| {
            if let Some(editor) = editor_handle.upgrade() {
                editor.update(cx, |editor, cx| {
                    f(editor, mode, cx);
                })
            } else {
                println!("Debug: editor handle could not be upgraded")
            }
        })
        .detach();
}

fn register(editor: &mut Editor, cx: &mut ViewContext<Editor>) {
    let initial_mode = match editor.mode() {
        editor::EditorMode::Full => "action",
        _ => "default",
    };
    switch_mode(editor, &SwitchMode(initial_mode.to_string()), cx);
    register_editor_action(editor, cx, select_line);
    register_editor_action(editor, cx, switch_mode);
    register_editor_action(editor, cx, paste_above);
    register_editor_action(editor, cx, paste_below);
}
