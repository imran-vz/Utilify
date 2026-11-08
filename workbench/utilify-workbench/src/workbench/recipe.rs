//! The recipe: one card per step with its arguments, in baking order.
//! Steps can be disabled, given a breakpoint, removed, or dragged into a
//! new place. Recipes copy out in CyberChef's chef format and paste in as
//! CyberChef JSON, chef format or a CyberChef URL.

use super::Workbench;
use super::operations::{DragLabel, DraggedOp};
use super::widgets::{
    Choice, chip, drop_line, dropdown, field_frame, hover_tint, panel, press_tint, remove_button,
    text_button, tool_button,
};
use gpui_kit::component::input::{Input, InputEvent, InputState, Textarea, TextareaState};
use gpui_kit::component::{Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use utilify_app::ui::{
    self, R_MD, R_SM, Tone,
    color::{self, *},
    px, radius, text,
};
use utilify_chef::recipe::Halt;
use utilify_chef::{ArgKind, ArgValue, Operation, Recipe, Step};

/// A step being dragged to a new place, by its id.
#[derive(Clone)]
pub struct DraggedStep(pub u64);

/// A step in the recipe and the text fields editing its arguments.
pub struct StepView {
    pub id: u64,
    pub step: Step,
    /// One per argument; `None` for arguments edited without a text field.
    fields: Vec<Option<Field>>,
    _subscriptions: Vec<Subscription>,
}

enum Field {
    Line(Entity<InputState>),
    Area(Entity<TextareaState>),
}

impl StepView {
    fn new(id: u64, step: Step, window: &mut Window, cx: &mut Context<Workbench>) -> Self {
        let mut fields = vec![];
        let mut subscriptions = vec![];
        for (i, (spec, value)) in step.op.args.iter().zip(&step.args).enumerate() {
            let text = match value {
                ArgValue::Str(s) | ArgValue::Toggle { string: s, .. } => s.clone(),
                ArgValue::Num(n) => utilify_chef::format_number(*n),
                ArgValue::Bool(_) => String::new(),
            };
            let field = match spec.kind {
                ArgKind::String { .. }
                | ArgKind::BinaryString { .. }
                | ArgKind::Number { .. }
                | ArgKind::Editable { .. }
                | ArgKind::Toggle { .. } => {
                    let state = cx.new(|cx| InputState::new(window, cx).default_value(text));
                    subscriptions.push(cx.subscribe_in(
                        &state,
                        window,
                        move |this, state, ev, window, cx| {
                            if matches!(ev, InputEvent::Change) {
                                let value = state.read(cx).value().to_string();
                                this.set_arg_text(id, i, value, window, cx);
                            }
                        },
                    ));
                    Some(Field::Line(state))
                }
                ArgKind::Text { .. } => {
                    let state = cx.new(|cx| {
                        TextareaState::new(window, cx)
                            .auto_grow(2, 8)
                            .soft_wrap(true)
                            .default_value(text)
                    });
                    subscriptions.push(cx.subscribe_in(
                        &state,
                        window,
                        move |this, state, ev, window, cx| {
                            if matches!(ev, InputEvent::Change) {
                                let value = state.read(cx).value().to_string();
                                this.set_arg_text(id, i, value, window, cx);
                            }
                        },
                    ));
                    Some(Field::Area(state))
                }
                ArgKind::Bool { .. } | ArgKind::Choice { .. } | ArgKind::Selector { .. } => None,
            };
            fields.push(field);
        }
        StepView {
            id,
            step,
            fields,
            _subscriptions: subscriptions,
        }
    }
}

impl Workbench {
    pub(super) fn push_step(
        &mut self,
        step: Step,
        at: Option<usize>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let id = self.next_step_id;
        self.next_step_id += 1;
        let view = StepView::new(id, step, window, cx);
        let at = at.unwrap_or(self.steps.len()).min(self.steps.len());
        self.steps.insert(at, view);
    }

    pub(super) fn add_op(
        &mut self,
        op: &'static Operation,
        at: Option<usize>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.push_step(Step::new(op), at, window, cx);
        self.recipe_changed(window, cx);
    }

    fn index_of(&self, id: u64) -> Option<usize> {
        self.steps.iter().position(|s| s.id == id)
    }

    fn update_step(
        &mut self,
        id: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
        f: impl FnOnce(&mut Step),
    ) {
        if let Some(i) = self.index_of(id) {
            f(&mut self.steps[i].step);
            self.recipe_changed(window, cx);
        }
    }

    /// A text field changed: store it as the argument's kind expects.
    fn set_arg_text(
        &mut self,
        id: u64,
        arg: usize,
        text: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.update_step(id, window, cx, |step| {
            let spec = step.op.args[arg];
            step.args[arg] = match (spec.kind, &step.args[arg]) {
                (ArgKind::Number { .. }, _) => match text.trim().parse() {
                    Ok(n) => ArgValue::Num(n),
                    Err(_) => ArgValue::Str(text),
                },
                (_, ArgValue::Toggle { format, .. }) => ArgValue::Toggle {
                    string: text,
                    format: format.clone(),
                },
                _ => ArgValue::Str(text),
            };
        });
    }

    fn set_arg(
        &mut self,
        id: u64,
        arg: usize,
        value: ArgValue,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.menu = None;
        self.update_step(id, window, cx, |step| step.args[arg] = value);
    }

    fn remove_step(&mut self, id: u64, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(i) = self.index_of(id) {
            self.steps.remove(i);
            self.recipe_changed(window, cx);
        }
    }

    /// Move step `id` to just before step `before` (or the end).
    fn move_step(
        &mut self,
        id: u64,
        before: Option<u64>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if Some(id) == before {
            return;
        }
        let Some(from) = self.index_of(id) else {
            return;
        };
        let view = self.steps.remove(from);
        let to = before
            .and_then(|b| self.index_of(b))
            .unwrap_or(self.steps.len());
        self.steps.insert(to, view);
        self.recipe_changed(window, cx);
    }

    /// Scroll the `i`th step into view (the output's error banner uses it).
    pub(super) fn show_step(&mut self, i: usize, cx: &mut Context<Self>) {
        self.recipe_scroll.scroll_to_item(i);
        cx.notify();
    }

    /// Disable the `i`th step, so the recipe bakes without it.
    pub(super) fn disable_step(&mut self, i: usize, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(id) = self.steps.get(i).map(|s| s.id) {
            self.update_step(id, window, cx, |s| s.disabled = true);
            // Bake even with auto bake off, so the failure clears.
            self.bake(None, window, cx);
        }
    }

    fn clear_recipe(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.steps.clear();
        self.recipe_changed(window, cx);
    }

    fn copy_recipe(&mut self, cx: &mut Context<Self>) {
        cx.write_to_clipboard(ClipboardItem::new_string(self.recipe().to_chef()));
        self.notify_user(Tone::Ok, "Recipe copied in chef format", cx);
    }

    /// Replace the recipe with one from the clipboard. A CyberChef URL's
    /// input comes along too.
    fn paste_recipe(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(text) = cx.read_from_clipboard().and_then(|c| c.text()) else {
            self.notify_user(Tone::Caution, "The clipboard has no text to paste", cx);
            return;
        };
        match Recipe::parse(&text) {
            Ok(recipe) if !recipe.steps.is_empty() => {
                self.steps.clear();
                for step in recipe.steps {
                    self.push_step(step, None, window, cx);
                }
                if let Some(input) = utilify_chef::recipe::url_input(&text) {
                    self.set_input_bytes("URL input".into(), input, window, cx);
                }
                self.notify_user(Tone::Ok, "Recipe pasted", cx);
                self.recipe_changed(window, cx);
            }
            Ok(_) => self.notify_user(
                Tone::Caution,
                "The clipboard has no recipe. Copy a CyberChef recipe, its chef format or URL first.",
                cx,
            ),
            Err(e) => self.notify_user(Tone::Danger, format!("Couldn't read the recipe: {e}"), cx),
        }
    }

    pub(super) fn recipe_panel(&self, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let n = self.steps.len();
        let caption = match n {
            0 => "no steps".to_string(),
            1 => "1 step".to_string(),
            n => format!("{n} steps"),
        };
        let actions = h_flex()
            .gap(px(6.))
            .child(
                text_button("paste-recipe", "Paste", true)
                    .tooltip(ui::tip(
                        "Paste a CyberChef recipe, chef format or URL",
                        None,
                    ))
                    .on_click(cx.listener(|this, _, window, cx| this.paste_recipe(window, cx))),
            )
            .child(
                text_button("copy-recipe", "Copy", n > 0)
                    .tooltip(ui::tip("Copy the recipe in chef format", None))
                    .when(n > 0, |b| {
                        b.on_click(cx.listener(|this, _, _, cx| this.copy_recipe(cx)))
                    }),
            )
            .child(
                tool_button("clear-recipe", "trash", fg2(), "Clear recipe", n > 0).when(
                    n > 0,
                    |b| {
                        b.on_click(cx.listener(|this, _, window, cx| this.clear_recipe(window, cx)))
                    },
                ),
            );
        let halt = self.result.as_ref().and_then(|o| o.halt.clone());
        let cards: Vec<AnyElement> = (0..n)
            .map(|i| {
                self.step_card(i, halt.as_ref(), window, cx)
                    .into_any_element()
            })
            .collect();
        let list = v_flex()
            .id("recipe-steps")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .track_scroll(&self.recipe_scroll)
            .p(px(12.))
            .gap(px(10.))
            .children(cards)
            .child(drop_zone(n == 0))
            .on_drop(
                cx.listener(|this, d: &DraggedOp, window, cx| this.add_op(d.0, None, window, cx)),
            )
            .on_drop(cx.listener(|this, d: &DraggedStep, window, cx| {
                this.move_step(d.0, None, window, cx)
            }));
        panel("Recipe", caption, actions, list)
    }

    fn step_card(
        &self,
        i: usize,
        halt: Option<&Halt>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let view = &self.steps[i];
        let (id, step) = (view.id, &view.step);
        let failed = matches!(halt, Some(Halt::Error { step, .. }) if *step == i);
        let paused = matches!(halt, Some(Halt::Breakpoint { step }) if *step == i);
        let border = if failed {
            danger()
        } else if paused {
            caution()
        } else {
            control_border()
        };
        let group = SharedString::from(format!("step-group-{id}"));
        let hidden: Vec<usize> = step
            .op
            .args
            .iter()
            .zip(&step.args)
            .flat_map(|(spec, v)| spec.hidden_by(v).iter().copied())
            .collect();

        let toggle_tint = |on: bool, color: Hsla| {
            move |b: Stateful<Div>| b.when(on, |b| b.bg(color.opacity(0.12)))
        };
        let header = h_flex()
            .gap(px(4.))
            .child(
                div()
                    .id(SharedString::from(format!("grip-{id}")))
                    .flex_none()
                    .h(px(28.))
                    .w(px(18.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(radius(R_SM))
                    .cursor_grab()
                    .hover(|s| s.bg(hover_tint()))
                    .tooltip(ui::tip("Drag to reorder", None))
                    .child(ui::icon("grip-vertical", 15., fg3()))
                    .on_drag(DraggedStep(id), {
                        let name = step.op.name;
                        move |_, _, _, cx| cx.new(|_| DragLabel(name.into()))
                    }),
            )
            .child(
                ui::mono(text::MICRO, if failed { danger() } else { fg3() })
                    .flex_none()
                    .min_w(px(18.))
                    .child((i + 1).to_string()),
            )
            .child(
                ui::ui_text(text::BODY, if step.disabled { fg3() } else { fg() })
                    .flex_1()
                    .min_w_0()
                    .font_weight(FontWeight::MEDIUM)
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .child(step.op.name),
            )
            .when(paused, |h| h.child(chip("paused", caution())))
            .when(failed, |h| h.child(chip("failed", danger())))
            .when(step.disabled, |h| h.child(chip("off", fg3())))
            .child(
                tool_button(
                    SharedString::from(format!("bp-{id}")),
                    "circle-pause",
                    if step.breakpoint { caution() } else { fg3() },
                    if step.breakpoint {
                        "Remove breakpoint"
                    } else {
                        "Pause before this step"
                    },
                    true,
                )
                .map(toggle_tint(step.breakpoint, caution()))
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.update_step(id, window, cx, |s| s.breakpoint = !s.breakpoint);
                })),
            )
            .child(
                tool_button(
                    SharedString::from(format!("off-{id}")),
                    if step.disabled { "eye-off" } else { "eye" },
                    if step.disabled { caution() } else { fg3() },
                    if step.disabled {
                        "Enable step"
                    } else {
                        "Disable step"
                    },
                    true,
                )
                .map(toggle_tint(step.disabled, caution()))
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.update_step(id, window, cx, |s| s.disabled = !s.disabled);
                })),
            )
            .child(
                remove_button(SharedString::from(format!("rm-{id}")), "Remove step").on_click(
                    cx.listener(move |this, _, window, cx| this.remove_step(id, window, cx)),
                ),
            );

        let error = match halt {
            Some(Halt::Error { step, message }) if *step == i => Some(message.clone()),
            _ => None,
        };

        let mut wide = v_flex().gap(px(10.));
        let mut flags = h_flex().flex_wrap().gap_x(px(18.)).gap_y(px(10.));
        let mut has_flags = false;
        for (a, spec) in step.op.args.iter().enumerate() {
            if hidden.contains(&a) {
                continue;
            }
            if let ArgKind::Bool { .. } = spec.kind {
                has_flags = true;
                let on = matches!(step.args[a], ArgValue::Bool(true));
                flags = flags.child(
                    h_flex()
                        .id(SharedString::from(format!("flag-{id}-{a}")))
                        .gap(px(9.))
                        .py(px(3.))
                        .pr(px(6.))
                        .rounded(radius(R_SM))
                        .cursor_pointer()
                        .hover(|s| s.bg(hover_tint()))
                        .active(|s| s.bg(press_tint()))
                        .child(ui::toggle(
                            SharedString::from(format!("flag-t-{id}-{a}")),
                            on,
                        ))
                        .child(ui::ui_text(text::SMALL, fg2()).child(spec.name))
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.set_arg(id, a, ArgValue::Bool(!on), window, cx);
                        })),
                );
            } else {
                wide = wide.child(
                    v_flex()
                        .gap(px(6.))
                        .child(ui::ui_text(text::CAPTION, fg2()).child(spec.name))
                        .child(self.arg_control(i, a, window, cx)),
                );
            }
        }

        v_flex()
            .id(SharedString::from(format!("step-{id}")))
            .group(group.clone())
            .relative()
            .flex_none()
            .p(px(12.))
            .gap(px(10.))
            .rounded(radius(R_MD))
            .bg(color::card())
            .border_1()
            .border_color(border)
            .when(failed, |c| c.bg(danger().opacity(0.05)))
            .when(step.disabled, |c| c.opacity(0.6))
            .drag_over::<DraggedOp>(|s, _, _, _| s.border_color(focus()))
            .drag_over::<DraggedStep>(|s, _, _, _| s.border_color(focus()))
            // Dropping here inserts above this step: show where.
            .child(drop_line::<DraggedOp, DraggedStep>(group))
            .on_drop(cx.listener(move |this, d: &DraggedOp, window, cx| {
                cx.stop_propagation();
                let at = this.index_of(id);
                this.add_op(d.0, at, window, cx);
            }))
            .on_drop(cx.listener(move |this, d: &DraggedStep, window, cx| {
                cx.stop_propagation();
                this.move_step(d.0, Some(id), window, cx);
            }))
            .child(header)
            .children(error.map(|e| {
                h_flex()
                    .items_start()
                    .gap(px(7.))
                    .child(
                        div()
                            .pt(px(2.))
                            .child(ui::icon(Tone::Danger.icon(), 14., danger())),
                    )
                    .child(
                        ui::ui_text(text::CAPTION, danger())
                            .flex_1()
                            .min_w_0()
                            .child(e),
                    )
            }))
            .when(!step.op.args.is_empty(), |c| c.child(wide))
            .when(has_flags, |c| c.child(flags))
    }

    /// The control for argument `a` of the `i`th step.
    fn arg_control(
        &self,
        i: usize,
        a: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let view = &self.steps[i];
        let id = view.id;
        let spec = view.step.op.args[a];
        let value = &view.step.args[a];
        let open = self.menu == Some((id, a));
        let menu_id = SharedString::from(format!("menu-{id}-{a}"));
        let toggle = cx.listener(move |this, _: &ClickEvent, _, cx| {
            this.menu = if this.menu == Some((id, a)) {
                None
            } else {
                Some((id, a))
            };
            cx.notify();
        });
        let dismiss = cx.listener(move |this, _: &MouseDownEvent, _, cx| {
            if this.menu == Some((id, a)) {
                this.menu = None;
                cx.notify();
            }
        });
        let current = match value {
            ArgValue::Str(s) => s.clone(),
            _ => String::new(),
        };
        let field = |fields: &[Option<Field>]| -> AnyElement {
            match &fields[a] {
                Some(Field::Line(s)) => field_frame(s.read(cx).focus_handle(cx).is_focused(window))
                    .bg(color::panel())
                    .child(
                        Input::new(s)
                            .appearance(false)
                            .small()
                            .text_size(px(text::SMALL)),
                    )
                    .into_any_element(),
                Some(Field::Area(s)) => field_frame(s.read(cx).focus_handle(cx).is_focused(window))
                    .bg(color::panel())
                    .py(px(4.))
                    .child(
                        Textarea::new(s)
                            .appearance(false)
                            .small()
                            .text_size(px(text::SMALL)),
                    )
                    .into_any_element(),
                None => div().into_any_element(),
            }
        };
        match spec.kind {
            ArgKind::Choice { options, .. } => {
                let choices = options
                    .iter()
                    .map(|&o| Choice {
                        label: o.into(),
                        selected: o == current,
                        on_pick: Box::new(cx.listener(move |this, _, window, cx| {
                            this.set_arg(id, a, ArgValue::Str(o.into()), window, cx);
                        })),
                    })
                    .collect();
                dropdown(menu_id, current, open, choices, toggle, dismiss).into_any_element()
            }
            ArgKind::Selector { options, .. } => {
                let choices = options
                    .iter()
                    .map(|o| {
                        let name = o.name;
                        Choice {
                            label: name.into(),
                            selected: name == current,
                            on_pick: Box::new(cx.listener(move |this, _, window, cx| {
                                this.set_arg(id, a, ArgValue::Str(name.into()), window, cx);
                            })),
                        }
                    })
                    .collect();
                dropdown(menu_id, current, open, choices, toggle, dismiss).into_any_element()
            }
            ArgKind::Editable { options, .. } => {
                // Presets fill the text field, which stays editable.
                let Some(Field::Line(state)) = &view.fields[a] else {
                    return div().into_any_element();
                };
                let preset = options
                    .iter()
                    .find(|(_, v)| *v == current)
                    .map_or("Custom", |(n, _)| *n);
                let choices = options
                    .iter()
                    .map(|&(name, v)| {
                        let state = state.clone();
                        Choice {
                            label: name.into(),
                            selected: v == current,
                            on_pick: Box::new(cx.listener(move |this, _, window, cx| {
                                this.menu = None;
                                // Setting the field fires no change event, so
                                // store the argument too.
                                state.update(cx, |s, cx| s.set_value(v.to_string(), window, cx));
                                this.set_arg(id, a, ArgValue::Str(v.into()), window, cx);
                            })),
                        }
                    })
                    .collect();
                v_flex()
                    .gap(px(6.))
                    .child(dropdown(menu_id, preset, open, choices, toggle, dismiss))
                    .child(field(&view.fields))
                    .into_any_element()
            }
            ArgKind::Toggle { formats, .. } => {
                let format = match value {
                    ArgValue::Toggle { format, .. } => format.clone(),
                    _ => formats.first().copied().unwrap_or_default().to_string(),
                };
                let choices = formats
                    .iter()
                    .map(|&f| Choice {
                        label: f.into(),
                        selected: f == format,
                        on_pick: Box::new(cx.listener(move |this, _, window, cx| {
                            this.menu = None;
                            this.update_step(id, window, cx, |s| {
                                if let ArgValue::Toggle { format, .. } = &mut s.args[a] {
                                    *format = f.into();
                                }
                            });
                        })),
                    })
                    .collect();
                h_flex()
                    .gap(px(6.))
                    .child(div().flex_1().min_w_0().child(field(&view.fields)))
                    .child(
                        div()
                            .w(px(120.))
                            .flex_none()
                            .child(dropdown(menu_id, format, open, choices, toggle, dismiss)),
                    )
                    .into_any_element()
            }
            _ => field(&view.fields),
        }
    }
}

/// The dashed target under the steps; fills the panel when it's empty.
fn drop_zone(empty: bool) -> Div {
    let zone = div()
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .rounded(radius(R_MD))
        .border_1()
        .border_dashed()
        .border_color(control_border())
        .text_color(fg3())
        .drag_over::<DraggedOp>(|s, _, _, _| {
            s.border_color(focus())
                .bg(focus().opacity(0.08))
                .text_color(fg())
        })
        .drag_over::<DraggedStep>(|s, _, _, _| {
            s.border_color(focus())
                .bg(focus().opacity(0.08))
                .text_color(fg())
        });
    if empty {
        zone.flex_1().min_h(px(200.)).child(ui::empty_state(
            "list-plus",
            "No steps yet",
            "Drag an operation here or click one on the left",
        ))
    } else {
        zone.h(px(56.))
            .gap(px(7.))
            .child(ui::icon("plus", 14., fg3()))
            .child(ui::ui_text(text::CAPTION, fg3()).child("Drop here to add at the end"))
    }
}
