//! Input and output. Input is typed text, or a file's raw bytes (opened or
//! dropped). Output shows the last bake as text, with its size and time; it
//! copies, saves to a file, or becomes the next input.

use super::widgets::{chip, panel, text_button, tool_button};
use super::{InputFile, MAX_SHOWN, Workbench, thousands};
use gpui_kit::component::h_flex;
use gpui_kit::component::input::Textarea;
use gpui_kit::component::v_flex;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use std::path::PathBuf;
use std::sync::Arc;
use utilify_app::ui::{self, R_MD, Tone, color::*, px, radius, text};
use utilify_chef::Dish;
use utilify_chef::recipe::Halt;
use utilify_core::format;

impl Workbench {
    /// Use `bytes` as the input in place of the text field.
    pub(super) fn set_input_bytes(
        &mut self,
        name: SharedString,
        bytes: Vec<u8>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.input_file = Some(InputFile {
            name,
            bytes: Arc::new(bytes),
        });
        self.input_changed(window, cx);
    }

    fn clear_input(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.input_file = None;
        self.input.update(cx, |s, cx| s.set_value("", window, cx));
        self.input_changed(window, cx);
    }

    pub(super) fn open_file(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("Use as Input".into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(paths))) = paths.await else {
                return;
            };
            if let Some(path) = paths.into_iter().next() {
                Self::load_file(this, path, cx).await;
            }
        })
        .detach();
    }

    fn drop_files(&mut self, paths: &ExternalPaths, window: &mut Window, cx: &mut Context<Self>) {
        let Some(path) = paths.paths().first().cloned() else {
            return;
        };
        cx.spawn_in(window, async move |this, cx| {
            Self::load_file(this, path, cx).await
        })
        .detach();
    }

    async fn load_file(this: WeakEntity<Self>, path: PathBuf, cx: &mut AsyncWindowContext) {
        let read = path.clone();
        let bytes = cx
            .background_executor()
            .spawn(async move { std::fs::read(read) })
            .await;
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "file".into());
        let _ = this.update_in(cx, |this, window, cx| match bytes {
            Ok(bytes) => this.set_input_bytes(name.into(), bytes, window, cx),
            Err(e) => this.notify_user(Tone::Danger, format!("Couldn't open {name}: {e}"), cx),
        });
    }

    /// Text output as UTF-8; byte output exactly as produced.
    fn output_bytes(&self) -> Option<Vec<u8>> {
        let out = self.result.as_ref()?;
        Some(match &out.dish {
            Dish::Bytes(b) => b.clone(),
            Dish::Text(t) => t.clone().into_bytes(),
            Dish::Html(h) => utilify_chef::dish::html_to_text(h).into_bytes(),
        })
    }

    fn copy_output(&mut self, cx: &mut Context<Self>) {
        let Some(out) = &self.result else { return };
        let text = match &out.dish {
            Dish::Html(h) => utilify_chef::dish::html_to_text(h),
            other => other.text().into_owned(),
        };
        cx.write_to_clipboard(ClipboardItem::new_string(text));
        self.notify_user(Tone::Ok, "Output copied", cx);
    }

    fn save_output(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(bytes) = self.output_bytes() else {
            return;
        };
        let dir = dirs_downloads();
        let path = cx.prompt_for_new_path(&dir, Some("output.dat"));
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(path))) = path.await else {
                return;
            };
            let target = path.clone();
            let written = cx
                .background_executor()
                .spawn(async move { std::fs::write(target, bytes) })
                .await;
            let name = path.file_name().map_or_else(
                || path.display().to_string(),
                |n| n.to_string_lossy().into_owned(),
            );
            let _ = this.update(cx, |this, cx| match written {
                Ok(()) => this.notify_user(Tone::Ok, format!("Saved {name}"), cx),
                Err(e) => this.notify_user(Tone::Danger, format!("Couldn't save {name}: {e}"), cx),
            });
        })
        .detach();
    }

    /// Make the output the next input, as CyberChef's "replace input with
    /// output" does. Binary output stays bytes.
    fn output_to_input(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(out) = &self.result else { return };
        match &out.dish {
            Dish::Bytes(b) if std::str::from_utf8(b).is_err() => {
                let bytes = b.clone();
                self.set_input_bytes("Previous output".into(), bytes, window, cx);
            }
            dish => {
                let text = match dish {
                    Dish::Html(h) => utilify_chef::dish::html_to_text(h),
                    other => other.text().into_owned(),
                };
                self.input_file = None;
                self.input.update(cx, |s, cx| s.set_value(text, window, cx));
                self.input_changed(window, cx);
            }
        }
    }

    pub(super) fn input_panel(&self, cx: &mut Context<Self>) -> Div {
        let caption = match &self.input_file {
            Some(f) => ui::dotted(vec![
                div().child(f.name.clone()).into_any_element(),
                div()
                    .child(format::bytes(f.bytes.len() as u64))
                    .into_any_element(),
            ]),
            None => {
                let text = self.input.read(cx).value();
                let lines = if text.is_empty() {
                    0
                } else {
                    text.matches('\n').count() + 1
                };
                ui::dotted(vec![
                    div()
                        .child(format!("length {}", thousands(text.chars().count())))
                        .into_any_element(),
                    div()
                        .child(format!("lines {}", thousands(lines)))
                        .into_any_element(),
                ])
            }
        };
        let has_input = self.input_file.is_some() || !self.input.read(cx).value().is_empty();
        let actions = h_flex()
            .gap(px(6.))
            .child(
                text_button("open-file", "Open file", true)
                    .tooltip(ui::tip("Open a file as input", Some("⌘O")))
                    .on_click(cx.listener(|this, _, window, cx| this.open_file(window, cx))),
            )
            .child(
                tool_button("clear-input", "trash", fg2(), "Clear input", has_input)
                    .when(has_input, |b| {
                        b.on_click(cx.listener(|this, _, window, cx| this.clear_input(window, cx)))
                    }),
            );
        let body = match &self.input_file {
            Some(f) => file_card(f, cx).into_any_element(),
            None => div()
                .flex_1()
                .min_h_0()
                .p(px(10.))
                .font_family(utilify_app::theme::MONO_FONT)
                .text_size(px(text::SMALL))
                .child(Textarea::new(&self.input).appearance(false).h_full())
                .into_any_element(),
        };
        panel(
            "Input",
            caption,
            actions,
            v_flex().flex_1().min_h_0().child(body),
        )
        .group(DROP_GROUP)
        .relative()
        .child(drop_overlay())
        .drag_over::<ExternalPaths>(|s, _, _, _| s.border_color(focus()))
        .on_drop(
            cx.listener(|this, paths: &ExternalPaths, window, cx| {
                this.drop_files(paths, window, cx)
            }),
        )
    }

    pub(super) fn output_panel(&self, cx: &mut Context<Self>) -> Div {
        let out = self.result.as_ref();
        let mut parts: Vec<AnyElement> = vec![];
        if let Some(o) = out {
            parts.push(
                div()
                    .child(format!("length {}", thousands(o.chars)))
                    .into_any_element(),
            );
            parts.push(
                div()
                    .child(format!("lines {}", thousands(o.lines)))
                    .into_any_element(),
            );
            parts.push(div().child(duration(o.duration)).into_any_element());
        }
        let caption = h_flex()
            .gap(px(8.))
            .child(ui::dotted(parts))
            .when(self.baking, |c| c.child(chip("baking", fg3())))
            .when(out.is_some_and(|o| o.dish.is_binary()), |c| {
                c.child(chip("binary", caution()))
            });
        let has_output = out.is_some_and(|o| !o.dish.is_empty());
        let actions = h_flex()
            .gap(px(6.))
            .child(
                text_button("to-input", "Use as input", has_output)
                    .tooltip(ui::tip("Replace the input with this output", None))
                    .when(has_output, |b| {
                        b.on_click(
                            cx.listener(|this, _, window, cx| this.output_to_input(window, cx)),
                        )
                    }),
            )
            .child(
                text_button("save-output", "Save", has_output)
                    .tooltip(ui::tip("Save the output to a file", None))
                    .when(has_output, |b| {
                        b.on_click(cx.listener(|this, _, window, cx| this.save_output(window, cx)))
                    }),
            )
            .child(
                text_button("copy-output", "Copy", has_output)
                    .tooltip(ui::tip("Copy the output", None))
                    .when(has_output, |b| {
                        b.on_click(cx.listener(|this, _, _, cx| this.copy_output(cx)))
                    }),
            );

        let step_name = |i: usize| self.steps.get(i).map_or("A step", |s| s.step.op.name);
        let banner = out.and_then(|o| match &o.halt {
            Some(Halt::Error { step, .. }) => {
                let i = *step;
                Some(ui::banner(
                    Tone::Danger,
                    format!("{} failed at step {}", step_name(i), i + 1),
                    vec![ui::note(
                        "Its error is shown below. Change its arguments, or disable it to bake without it.",
                    )],
                    vec![
                        ui::banner_button("show-failed-step", "Show step")
                            .on_click(cx.listener(move |this, _, _, cx| this.show_step(i, cx))),
                        ui::banner_button("disable-failed-step", "Disable step").on_click(
                            cx.listener(move |this, _, window, cx| this.disable_step(i, window, cx)),
                        ),
                    ],
                ))
            }
            Some(Halt::Breakpoint { step }) => {
                let i = *step;
                Some(ui::banner(
                    Tone::Caution,
                    format!("Paused before {}, step {}", step_name(i), i + 1),
                    vec![ui::note(
                        "The output is what the steps above it produced. Step runs on from here.",
                    )],
                    vec![
                        ui::banner_button("step", "Step")
                            .tooltip(ui::tip("Run the rest of the recipe", None))
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.step_past_breakpoint(window, cx)
                            })),
                        ui::banner_button("show-paused-step", "Show step")
                            .on_click(cx.listener(move |this, _, _, cx| this.show_step(i, cx))),
                    ],
                ))
            }
            None if o.truncated => Some(ui::banner(
                Tone::Info,
                format!("Showing the first {}", format::bytes(MAX_SHOWN as u64)),
                vec![ui::note("Copy and Save take all of it.")],
                vec![],
            )),
            None => None,
        });
        let failed = matches!(out.and_then(|o| o.halt.as_ref()), Some(Halt::Error { .. }));
        let empty = match out {
            None => Some(if self.baking {
                ("loader", "Baking", "The output shows here when it's done")
            } else {
                ("file-output", "No output yet", "Bake to see the output")
            }),
            Some(o) if o.dish.is_empty() && o.halt.is_none() => {
                let no_input = self.input_file.is_none() && self.input.read(cx).value().is_empty();
                Some(if no_input && self.steps.is_empty() {
                    (
                        "file-output",
                        "Nothing to bake yet",
                        "Type or open some input, then add operations to the recipe",
                    )
                } else if no_input {
                    (
                        "file-output",
                        "No input",
                        "Type or paste input above, or drop a file on it",
                    )
                } else {
                    (
                        "file-output",
                        "The output is empty",
                        "The recipe ran and produced nothing",
                    )
                })
            }
            Some(_) => None,
        };
        let body = match empty {
            Some((icon, title, detail)) => div()
                .flex_1()
                .min_h_0()
                .flex()
                .items_center()
                .justify_center()
                .px(px(20.))
                .child(ui::empty_state(icon, title, detail))
                .into_any_element(),
            None => div()
                .flex_1()
                .min_h_0()
                .p(px(10.))
                .font_family(utilify_app::theme::MONO_FONT)
                .text_size(px(text::SMALL))
                .when(failed, |d| d.text_color(danger()))
                .child(
                    Textarea::new(&self.output)
                        .appearance(false)
                        .readonly(true)
                        .h_full(),
                )
                .into_any_element(),
        };
        panel(
            "Output",
            caption,
            actions,
            v_flex()
                .flex_1()
                .min_h_0()
                .children(banner.map(|b| b.flex_none().mx(px(10.)).mt(px(10.))))
                .child(body),
        )
    }
}

/// Group name of the input panel, for the drop overlay.
const DROP_GROUP: &str = "input-drop";

/// Shown over the input while a file is dragged over it.
fn drop_overlay() -> Div {
    div()
        .absolute()
        .inset_0()
        .m(px(6.))
        .flex()
        .items_center()
        .justify_center()
        .rounded(radius(R_MD))
        .border_1()
        .border_dashed()
        .border_color(focus())
        // Nearly opaque, to hide the text under it.
        .bg(utilify_app::ui::color::panel().opacity(0.94))
        .invisible()
        // Group drag styles need a hitbox; a hover style gives it one.
        .hover(|s| s)
        .group_drag_over::<ExternalPaths>(DROP_GROUP, |s| s.visible())
        .child(ui::empty_state(
            "file-down",
            "Drop to use as input",
            "Its bytes are used as they are, without decoding",
        ))
}

fn file_card(f: &InputFile, cx: &mut Context<Workbench>) -> Div {
    v_flex()
        .flex_1()
        .items_center()
        .justify_center()
        .gap(px(10.))
        .px(px(20.))
        .child(ui::icon("file", 32., fg3()))
        .child(
            ui::ui_text(text::BODY, fg())
                .font_weight(FontWeight::MEDIUM)
                .child(f.name.clone()),
        )
        .child(ui::mono(text::CAPTION, fg3()).child(format!(
            "{} · {} bytes",
            format::bytes(f.bytes.len() as u64),
            thousands(f.bytes.len())
        )))
        .child(
            text_button("type-input", "Type input instead", true)
                .mt(px(4.))
                .on_click(cx.listener(|this, _, window, cx| this.clear_input(window, cx))),
        )
}

/// `4 ms`, `1.2 s`.
fn duration(d: std::time::Duration) -> String {
    let ms = d.as_secs_f64() * 1000.0;
    if ms < 1.0 {
        "<1 ms".into()
    } else if ms < 1000.0 {
        format!("{ms:.0} ms")
    } else {
        format!("{:.1} s", ms / 1000.0)
    }
}

fn dirs_downloads() -> PathBuf {
    std::env::var_os("HOME")
        .map(|h| PathBuf::from(h).join("Downloads"))
        .unwrap_or_else(std::env::temp_dir)
}
