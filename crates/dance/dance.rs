use editor::actions::{Backspace, NewlineAbove, NewlineBelow, Paste};
use editor::scroll::Autoscroll;
use editor::{Editor, EditorEvent, RowExt, RowRangeExt};
use gpui::{
    actions, impl_actions, AppContext, ClipboardEntry, IntoElement, Render, Subscription,
    ViewContext, VisualContext, WindowContext,
};
use gpui::{Action, KeyContext};
use gpui::{View, WeakView};
use language::{CursorShape, Point};
use multi_buffer::{MultiBufferRow, ToPoint};
use serde::Deserialize;
use std::iter::Iterator;
use std::ops::Range;
use text::SelectionGoal;

#[derive(Clone, Deserialize, PartialEq)]
struct SwitchMode(String);

impl_actions!(dance, [SwitchMode,]);
actions!(
    dance,
    [
        SelectLine,
        PasteAbove,
        PasteBelow,
        JoinLines,
        MoveToBeginningOfLine,
        MoveToEndOfLine,
    ]
);

pub(crate) struct Dance {
    dance_mode: String,
    editor: WeakView<Editor>,
    _subscriptions: Vec<Subscription>,
}

// Hack: Dance intercepts events dispatched to a window and updates the view in response.
// This means it needs a VisualContext. The easiest way to satisfy that constraint is
// to make Dance a "View" that is just never actually rendered.
impl Render for Dance {
    fn render(&mut self, _cx: &mut ViewContext<Self>) -> impl IntoElement {
        gpui::Empty
    }
}

#[derive(Clone)]
pub(crate) struct DanceAddon {
    pub(crate) view: View<Dance>,
}

impl editor::Addon for DanceAddon {
    fn extend_key_context(&self, key_context: &mut KeyContext, cx: &AppContext) {
        let dance_mode = &self.view.read(cx).dance_mode;
        key_context.set("dance_mode", dance_mode.to_string())
    }

    fn to_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// Initializes the `vim` crate.
pub fn init(cx: &mut AppContext) {
    cx.observe_new_views(|editor: &mut Editor, cx| register(editor, cx))
        .detach();
}

/// this is a custom implementation of line selection:
/// - it places the caret at the beginning, which looks nicer
/// - it don't extend the selection to the subsequent line if the selection has nonzero length
///   AND the end of the selection sits at the very start of the next line AND the selection caret
///   is at the beginning of the selection. this makes the operation idempotent but also behaves like
///   how a user might expect
fn select_line(
    _dance: &mut Dance,
    editor: &mut Editor,
    _: &SelectLine,
    cx: &mut ViewContext<Editor>,
) {
    let display_map = editor.display_map.update(cx, |map, cx| map.snapshot(cx));
    let mut selections = editor.selections.all::<Point>(cx);
    let max_point = display_map.buffer_snapshot.max_point();
    for selection in &mut selections {
        let rows = {
            let start = selection.start.to_point(&display_map.buffer_snapshot);
            let mut end = selection.end.to_point(&display_map.buffer_snapshot);
            if start.row != end.row && end.column == 0 && selection.reversed == true {
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

/// this is a custom implementation of paste that, if the clipboard contains a newline,
/// it will paste on a newly created line above the selection instead of replacing
/// the selection
fn paste_above(
    _dance: &mut Dance,
    editor: &mut Editor,
    _: &PasteAbove,
    cx: &mut ViewContext<Editor>,
) {
    let ends_in_newline = clipboard_ends_in_newline(cx);
    if ends_in_newline {
        editor.newline_above(&NewlineAbove, cx);
    }
    editor.paste(&Paste, cx);
    if ends_in_newline {
        editor.backspace(&Backspace, cx);
    }
}

/// this is a custom implementation of paste that, if the clipboard contains a newline,
/// it will paste on a newly created line below the selection instead of replacing
/// the selection
fn paste_below(
    _dance: &mut Dance,
    editor: &mut Editor,
    _: &PasteBelow,
    cx: &mut ViewContext<Editor>,
) {
    let ends_in_newline = clipboard_ends_in_newline(cx);
    if ends_in_newline {
        editor.newline_below(&NewlineBelow, cx);
    }
    editor.paste(&Paste, cx);
    if ends_in_newline {
        editor.backspace(&Backspace, cx);
    }
}

/// A custom implementation of join_lines that selects the space between lines
fn join_lines(
    _dance: &mut Dance,
    editor: &mut Editor,
    _: &JoinLines,
    cx: &mut ViewContext<Editor>,
) {
    if editor.read_only(cx) {
        return;
    }
    let mut row_ranges = Vec::<Range<MultiBufferRow>>::new();
    for selection in editor.selections.all::<Point>(cx) {
        let start = MultiBufferRow(selection.start.row);
        let end = if selection.start.row == selection.end.row {
            MultiBufferRow(selection.start.row + 1)
        } else {
            MultiBufferRow(selection.end.row)
        };

        if let Some(last_row_range) = row_ranges.last_mut() {
            if start <= last_row_range.end {
                last_row_range.end = end;
                continue;
            }
        }
        row_ranges.push(start..end);
    }

    let snapshot = editor.buffer().read(cx).snapshot(cx);
    editor.transact(cx, |this, cx| {
        let mut cursor_positions = Vec::new();
        // for row_range in row_ranges.iter().rev() {
        //     println!("{:?}   {:?}", row_range, row_range.end.previous_row());
        //     let anchor = snapshot.anchor_before(Point::new(
        //         row_range.end.previous_row().0,
        //         snapshot.line_len(row_range.end.previous_row()),
        //     ));
        //     cursor_positions.push(anchor..anchor);
        // }

        for row_range in row_ranges.into_iter().rev() {
            for row in row_range.iter_rows().rev() {
                {
                    let start = snapshot.anchor_before(Point::new(row.0, snapshot.line_len(row)));
                    let end = snapshot.anchor_before(Point::new(row.next_row().0, 0));
                    cursor_positions.push(start..end);
                }

                let end_of_line = Point::new(row.0, snapshot.line_len(row));
                let next_line_row = row.next_row();
                let indent = snapshot.indent_size_for_line(next_line_row);
                let start_of_next_line = Point::new(next_line_row.0, indent.len);

                let replace = if snapshot.line_len(next_line_row) > indent.len {
                    " "
                } else {
                    ""
                };

                this.buffer().update(cx, |buffer, cx| {
                    buffer.edit([(end_of_line..start_of_next_line, replace)], None, cx)
                });
            }
        }

        // it's important that cursor positions are in increasing order
        cursor_positions.reverse();

        this.change_selections(Some(Autoscroll::fit()), cx, |s| {
            s.select_anchor_ranges(cursor_positions)
        });
    });
}

fn switch_mode(
    dance: &mut Dance,
    editor: &mut Editor,
    &SwitchMode(ref mode): &SwitchMode,
    cx: &mut ViewContext<Editor>,
) {
    dance.dance_mode = mode.to_string();
    sync(mode, editor, cx);
}

fn sync(dance_mode: &str, editor: &mut Editor, cx: &mut ViewContext<Editor>) {
    if dance_mode == "default" {
        editor.set_cursor_shape(CursorShape::Bar, cx);
    } else {
        editor.set_cursor_shape(CursorShape::WideBar, cx);
    }
}

fn all_selections_are_empty(editor: &Editor, cx: &mut AppContext) -> bool {
    editor
        .selections
        .all::<usize>(cx)
        .iter()
        .all(|s| s.is_empty())
}

fn move_to_beginning_of_line(
    _dance: &mut Dance,
    editor: &mut Editor,
    _: &MoveToBeginningOfLine,
    cx: &mut ViewContext<Editor>,
) {
    if all_selections_are_empty(editor, cx) {
        editor.move_to_beginning_of_line(
            &editor::actions::MoveToBeginningOfLine {
                stop_at_soft_wraps: true,
            },
            cx,
        )
    } else {
        editor.change_selections(Some(Autoscroll::fit()), cx, |s| {
            s.move_with(|_, selection| {
                selection.collapse_to(selection.start, SelectionGoal::None);
            });
        })
    }
}

fn move_to_end_of_line(
    _dance: &mut Dance,
    editor: &mut Editor,
    _: &MoveToEndOfLine,
    cx: &mut ViewContext<Editor>,
) {
    if all_selections_are_empty(editor, cx) {
        editor.move_to_end_of_line(
            &editor::actions::MoveToEndOfLine {
                stop_at_soft_wraps: true,
            },
            cx,
        )
    } else {
        editor.change_selections(Some(Autoscroll::fit()), cx, |s| {
            s.move_with(|_, selection| {
                selection.collapse_to(selection.end, SelectionGoal::None);
            });
        })
    }
}

fn register_editor_action<T: Action>(
    editor: &mut Editor,
    cx: &mut ViewContext<Dance>,
    f: fn(&mut Dance, &mut Editor, &T, &mut ViewContext<Editor>),
) {
    let dance_handle = cx.view().downgrade();
    editor
        .register_action::<T>(move |mode, cx: &mut WindowContext| {
            dance_handle
                .update(cx, |dance, cx| {
                    let Some(editor) = dance.editor.upgrade() else {
                        return;
                    };
                    editor.update(cx, |editor, cx| {
                        f(dance, editor, mode, cx);
                    });
                })
                .unwrap();
        })
        .detach();
}

fn handle_editor_event<'a>(
    this: &mut Dance,
    editor: View<Editor>,
    event: &EditorEvent,
    cx: &mut ViewContext<'a, Dance>,
) {
    match event {
        EditorEvent::Focused | EditorEvent::FocusedIn => {
            editor.update(cx, |editor, cx| {
                sync(&this.dance_mode, editor, cx);
            });
        }
        _ => {}
    }
}

fn register(editor: &mut Editor, cx: &mut ViewContext<Editor>) {
    let initial_mode = match editor.mode() {
        editor::EditorMode::Full => "action",
        _ => "default",
    };
    let editor_weak = cx.view().clone().downgrade();
    let editor_view = cx.view().clone();
    let dance = cx.new_view(|cx| Dance {
        editor: editor_weak,
        dance_mode: initial_mode.to_string(),
        _subscriptions: vec![cx.subscribe(&editor_view, handle_editor_event)],
    });
    editor.register_addon(DanceAddon {
        view: dance.clone(),
    });
    sync(initial_mode, editor, cx);
    dance.update(cx, |_dance, cx| {
        register_editor_action(editor, cx, select_line);
        register_editor_action(editor, cx, switch_mode);
        register_editor_action(editor, cx, paste_above);
        register_editor_action(editor, cx, paste_below);
        register_editor_action(editor, cx, move_to_beginning_of_line);
        register_editor_action(editor, cx, move_to_end_of_line);
        register_editor_action(editor, cx, join_lines);
    })
}
