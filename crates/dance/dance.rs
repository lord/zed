use anyhow::Result;
use collections::HashMap;
use command_palette_hooks::{CommandPaletteFilter, CommandPaletteInterceptor};
use editor::{
    movement::{self, FindRange},
    Anchor, Bias, Editor, EditorEvent, EditorMode, ToPoint,
};
use gpui::KeyContext;
use gpui::{
    actions, impl_actions, Action, AppContext, EntityId, FocusableView, Global, KeystrokeEvent,
    Subscription, UpdateGlobal, View, ViewContext, WeakView, WindowContext,
};
use language::{CursorShape, Point, SelectionGoal, TransactionId};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_derive::Serialize;
use settings::{update_settings_file, Settings, SettingsSources, SettingsStore};
use std::{ops::Range, sync::Arc};
use ui::BorrowAppContext;
use workspace::{self, Workspace};

struct DanceTag;

#[derive(Clone, Deserialize, PartialEq)]
struct SwitchMode(String);

impl_actions!(dance, [SwitchMode,]);

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

fn register(editor: &mut Editor, cx: &mut ViewContext<Editor>) {
    let editor_handle = cx.view().downgrade();
    editor.set_keymap_context_layer::<DanceTag>(make_key_context("default".to_string()), cx);
    editor.set_cursor_shape(CursorShape::WideBar, cx);
    editor
        .register_action(
            move |&SwitchMode(ref mode): &SwitchMode, cx: &mut WindowContext| {
                if let Some(editor) = editor_handle.upgrade() {
                    editor.update(cx, |editor, cx| {
                        editor.set_keymap_context_layer::<DanceTag>(
                            make_key_context(mode.to_string()),
                            cx,
                        );
                        if mode == "default" {
                            editor.set_cursor_shape(CursorShape::Bar, cx);
                        } else {
                            editor.set_cursor_shape(CursorShape::WideBar, cx);
                        }
                    })
                }
            },
        )
        .detach();
}
