use editor::actions::{Backspace, NewlineAbove, NewlineBelow, Paste};
use editor::scroll::Autoscroll;

use editor::display_map::DisplaySnapshot;
use editor::{Editor, EditorEvent, RowExt, RowRangeExt, SelectionEffects};
use gpui::WeakEntity;
use gpui::{
    actions, App, AppContext, ClipboardEntry, Context, Entity, IntoElement, Render, Subscription,
    Window,
};
use schemars::JsonSchema;

use gpui::{Action, KeyContext};
use language::{CursorShape, Point};
use multi_buffer::{MultiBufferRow, ToPoint};
use serde::Deserialize;
use std::iter::Iterator;
use std::ops::Range;
use text::SelectionGoal;

#[derive(Clone, Deserialize, PartialEq, JsonSchema, Action)]
#[action(namespace = dance)]
struct SwitchMode(String);

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
    editor: WeakEntity<Editor>,
    _subscriptions: Vec<Subscription>,
}

impl Render for Dance {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        gpui::Empty
    }
}
#[derive(Clone)]
pub(crate) struct DanceAddon {
    pub(crate) view: Entity<Dance>,
}

impl editor::Addon for DanceAddon {
    fn extend_key_context(&self, key_context: &mut KeyContext, cx: &App) {
        let dance_mode = &self.view.read(cx).dance_mode;
        key_context.set("dance_mode", dance_mode.to_string())
    }

    fn to_any(&self) -> &dyn std::any::Any {
        self
    }
}

pub fn init(cx: &mut App) {
    cx.observe_new(
        |editor: &mut Editor, window: Option<&mut Window>, cx: &mut Context<Editor>| {
            let Some(window) = window else {
                return;
            };

            let initial_mode = match editor.mode() {
                editor::EditorMode::Full { .. } => "action",
                _ => "default",
            };
            let editor_weak = cx.weak_entity();
            let dance = {
                let editor = cx.entity().clone();
                cx.new(|cx| Dance {
                    editor: editor_weak,
                    dance_mode: initial_mode.to_string(),
                    _subscriptions: vec![cx.subscribe_in(
                        &editor,
                        window,
                        |this: &mut Dance, _, event, window, cx| {
                            handle_editor_event(this, event, window, cx)
                        },
                    )],
                })
            };
            editor.register_addon(DanceAddon {
                view: dance.clone(),
            });
            sync(initial_mode, editor, window, cx);

            dance.update(cx, |_dance, cx| {
                register_editor_action(editor, cx, select_line);
                register_editor_action(editor, cx, switch_mode);
                register_editor_action(editor, cx, paste_above);
                register_editor_action(editor, cx, paste_below);
                register_editor_action(editor, cx, move_to_beginning_of_line);
                register_editor_action(editor, cx, move_to_end_of_line);
                register_editor_action(editor, cx, join_lines);
            })
        },
    )
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
    window: &mut Window,
    cx: &mut Context<Editor>,
) {
    let display_map = editor.display_map.update(cx, |map, cx| map.snapshot(cx));
    let mut selections = editor.selections.all::<Point>(&display_map);
    let max_point = display_map.buffer_snapshot().max_point();
    for selection in &mut selections {
        let rows = {
            let start = selection.start.to_point(&display_map.buffer_snapshot());
            let mut end = selection.end.to_point(&display_map.buffer_snapshot());
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

    editor.change_selections(
        SelectionEffects::scroll(Autoscroll::fit()),
        window,
        cx,
        |s| {
            s.select(selections);
        },
    );
}

fn clipboard_ends_in_newline(cx: &mut Context<Editor>) -> bool {
    if let Some(item) = cx.read_from_clipboard() {
        item.entries().len() > 0
            && item.entries().iter().all(|entry| match entry {
                ClipboardEntry::Image(_) | ClipboardEntry::ExternalPaths(_) => false,
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
    window: &mut Window,
    cx: &mut Context<Editor>,
) {
    let ends_in_newline = clipboard_ends_in_newline(cx);
    if ends_in_newline {
        editor.newline_above(&NewlineAbove, window, cx);
    }
    editor.paste(&Paste, window, cx);
    if ends_in_newline {
        editor.backspace(&Backspace, window, cx);
    }
}

/// this is a custom implementation of paste that, if the clipboard contains a newline,
/// it will paste on a newly created line below the selection instead of replacing
/// the selection
fn paste_below(
    _dance: &mut Dance,
    editor: &mut Editor,
    _: &PasteBelow,
    window: &mut Window,
    cx: &mut Context<Editor>,
) {
    let ends_in_newline = clipboard_ends_in_newline(cx);
    if ends_in_newline {
        editor.newline_below(&NewlineBelow, window, cx);
    }
    editor.paste(&Paste, window, cx);
    if ends_in_newline {
        editor.backspace(&Backspace, window, cx);
    }
}

/// A custom implementation of join_lines that selects the space between lines
fn join_lines(
    _dance: &mut Dance,
    editor: &mut Editor,
    _: &JoinLines,
    window: &mut Window,
    cx: &mut Context<Editor>,
) {
    if editor.read_only(cx) {
        return;
    }
    let mut row_ranges = Vec::<Range<MultiBufferRow>>::new();
    let display_map = editor.display_map.update(cx, |map, cx| map.snapshot(cx));
    for selection in editor.selections.all::<Point>(&display_map) {
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
    editor.transact(window, cx, |this, window, cx| {
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

        this.change_selections(
            SelectionEffects::scroll(Autoscroll::fit()),
            window,
            cx,
            |s| s.select_anchor_ranges(cursor_positions),
        );
    });
}

fn switch_mode(
    dance: &mut Dance,
    editor: &mut Editor,
    &SwitchMode(ref mode): &SwitchMode,
    window: &mut Window,
    cx: &mut Context<Editor>,
) {
    dance.dance_mode = mode.to_string();
    sync(mode, editor, window, cx);
}

fn sync(dance_mode: &str, editor: &mut Editor, _window: &mut Window, cx: &mut Context<Editor>) {
    if dance_mode == "default" {
        editor.set_cursor_shape(CursorShape::Bar, cx);
    } else {
        editor.set_cursor_shape(CursorShape::WideBar, cx);
    }
}

fn all_selections_are_empty(editor: &Editor, snapshot: &DisplaySnapshot) -> bool {
    editor
        .selections
        .all::<Point>(snapshot)
        .iter()
        .all(|s| s.is_empty())
}

fn move_to_beginning_of_line(
    _dance: &mut Dance,
    editor: &mut Editor,
    _: &MoveToBeginningOfLine,
    window: &mut Window,
    cx: &mut Context<Editor>,
) {
    let display_map = editor.display_map.update(cx, |map, cx| map.snapshot(cx));
    if all_selections_are_empty(editor, &display_map) {
        editor.move_to_beginning_of_line(
            &editor::actions::MoveToBeginningOfLine {
                stop_at_soft_wraps: true,
                stop_at_indent: true,
            },
            window,
            cx,
        )
    } else {
        editor.change_selections(
            SelectionEffects::scroll(Autoscroll::fit()),
            window,
            cx,
            |s| {
                s.move_with(&mut |_, selection| {
                    selection.collapse_to(selection.start, SelectionGoal::None);
                });
            },
        )
    }
}

fn move_to_end_of_line(
    _dance: &mut Dance,
    editor: &mut Editor,
    _: &MoveToEndOfLine,
    window: &mut Window,
    cx: &mut Context<Editor>,
) {
    let display_map = editor.display_map.update(cx, |map, cx| map.snapshot(cx));
    if all_selections_are_empty(editor, &display_map) {
        editor.move_to_end_of_line(
            &editor::actions::MoveToEndOfLine {
                stop_at_soft_wraps: true,
            },
            window,
            cx,
        )
    } else {
        editor.change_selections(
            SelectionEffects::scroll(Autoscroll::fit()),
            window,
            cx,
            |s| {
                s.move_with(&mut |_, selection| {
                    selection.collapse_to(selection.end, SelectionGoal::None);
                });
            },
        )
    }
}

fn register_editor_action<T: Action>(
    editor: &mut Editor,
    cx: &mut Context<Dance>,
    f: fn(&mut Dance, &mut Editor, &T, &mut Window, &mut Context<Editor>),
) {
    let dance_handle = cx.entity().downgrade();
    editor
        .register_action::<T>(move |mode, window: &mut Window, app: &mut App| {
            dance_handle
                .update(app, |dance, cx| {
                    let Some(editor) = dance.editor.upgrade() else {
                        return;
                    };
                    editor.update(cx, |editor, cx| {
                        f(dance, editor, mode, window, cx);
                    });
                })
                .unwrap();
        })
        .detach();
}

fn handle_editor_event<'a>(
    this: &mut Dance,
    event: &EditorEvent,
    window: &mut Window,
    cx: &mut Context<Dance>,
) {
    match event {
        EditorEvent::Focused | EditorEvent::FocusedIn => {
            this.editor
                .update(cx, |editor, cx| {
                    sync(&this.dance_mode, editor, window, cx);
                })
                .unwrap();
        }
        _ => {}
    }
}
