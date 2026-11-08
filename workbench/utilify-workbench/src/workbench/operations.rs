//! The operations list: search, favourites, and CyberChef's categories.
//! Click an operation to append it to the recipe, or drag it to a place in
//! the recipe. Operations not ported yet are listed dimmed.

use super::Workbench;
use super::widgets::{chip, field_frame, hover_tint, panel, press_tint, tool_button};
use gpui_kit::component::input::Input;
use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use utilify_app::ui::{self, R_MD, R_SM, color::*, px, radius, text};
use utilify_chef::Operation;
use utilify_chef::catalog::CATEGORIES;

/// An operation being dragged from the list.
#[derive(Clone)]
pub struct DraggedOp(pub &'static Operation);

/// What follows the pointer while dragging.
pub struct DragLabel(pub SharedString);

impl Render for DragLabel {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        h_flex()
            .gap(px(8.))
            .px(px(12.))
            .py(px(7.))
            .rounded(radius(R_MD))
            .bg(popover())
            .border_1()
            .border_color(focus())
            .shadow(vec![BoxShadow {
                color: shadow(),
                offset: point(px(0.), px(6.)),
                blur_radius: px(18.),
                spread_radius: px(0.),
                inset: false,
            }])
            .font_family(utilify_app::theme::UI_FONT)
            .text_size(px(text::SMALL))
            .text_color(fg())
            .child(ui::icon("grip-vertical", 14., fg3()))
            .child(self.0.clone())
    }
}

impl Workbench {
    pub(super) fn operations_panel(&self, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let list = if self.query.is_empty() {
            self.category_list(cx)
        } else {
            self.search_results(cx)
        };
        let ported = utilify_chef::operations().len();
        let focused = self.search.read(cx).focus_handle(cx).is_focused(window);
        let searching = !self.query.is_empty();
        let search = field_frame(focused)
            .pl(px(10.))
            .gap(px(4.))
            .font_family(utilify_app::theme::UI_FONT)
            .child(ui::icon("search", 14., if focused { fg2() } else { fg3() }))
            .child(
                div().flex_1().min_w_0().child(
                    Input::new(&self.search)
                        .appearance(false)
                        .text_size(px(text::SMALL)),
                ),
            )
            .when(searching, |f| {
                f.child(
                    tool_button("clear-search", "x", fg3(), "Clear search", true)
                        .size(px(26.))
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.search.update(cx, |s, cx| s.set_value("", window, cx));
                            this.query.clear();
                            cx.notify();
                        })),
                )
            })
            .when(!searching && !focused, |f| {
                f.child(ui::mono(text::MICRO, fg4()).pr(px(8.)).child("⌘F"))
            });
        panel(
            "Operations",
            format!("{ported} ported"),
            div(),
            v_flex()
                .flex_1()
                .min_h_0()
                .child(div().flex_none().p(px(12.)).child(search))
                .child(
                    v_flex()
                        .id("operations-list")
                        .flex_1()
                        .min_h_0()
                        .overflow_y_scroll()
                        .px(px(8.))
                        .pb(px(8.))
                        .child(list),
                )
                .child(self.description_box()),
        )
    }

    fn category_list(&self, cx: &mut Context<Self>) -> Div {
        let mut list = v_flex().gap(px(2.));
        let favourites = self.session.favourite_ops();
        list = list.child(self.category_header("Favourites", favourites.len(), None, cx));
        if self.expanded.contains("Favourites") {
            if favourites.is_empty() {
                list = list.child(
                    h_flex()
                        .h(px(30.))
                        .pl(px(29.))
                        .gap(px(6.))
                        .child(ui::icon("star", 13., fg4()))
                        .child(
                            ui::ui_text(text::CAPTION, fg3())
                                .child("Star an operation to keep it here"),
                        ),
                );
            }
            list = list.children(favourites.iter().map(|op| self.op_row("fav", op, cx)));
        }
        for &(name, names) in CATEGORIES {
            let ops: Vec<&'static Operation> =
                names.iter().filter_map(|n| utilify_chef::find(n)).collect();
            list = list.child(self.category_header(name, ops.len(), Some(names.len()), cx));
            if self.expanded.contains(name) {
                for n in names {
                    list = list.child(match utilify_chef::find(n) {
                        Some(op) => self.op_row(name, op, cx).into_any_element(),
                        None => unported_row(n).into_any_element(),
                    });
                }
            }
        }
        list
    }

    /// Name and description matches, names first; then catalog names not
    /// ported yet.
    fn search_results(&self, cx: &mut Context<Self>) -> Div {
        let q = self.query.as_str();
        let ops = utilify_chef::operations();
        let by_name = ops.iter().filter(|op| op.name.to_lowercase().contains(q));
        let by_desc = ops.iter().filter(|op| {
            !op.name.to_lowercase().contains(q) && op.description.to_lowercase().contains(q)
        });
        let rows: Vec<_> = by_name
            .chain(by_desc)
            .map(|op| self.op_row("search", op, cx))
            .collect();
        let pending: Vec<&str> = CATEGORIES
            .iter()
            .flat_map(|(_, names)| names.iter().copied())
            .filter(|n| utilify_chef::find(n).is_none() && n.to_lowercase().contains(q))
            .collect();
        let empty = rows.is_empty() && pending.is_empty();
        let shown = self.search.read(cx).value().trim().to_string();
        v_flex()
            .gap(px(2.))
            .children(rows)
            .children(pending.into_iter().map(unported_row))
            .when(empty, |v| {
                v.child(ui::empty_state(
                    "search-x",
                    &format!("No operations match “{shown}”"),
                    "Try a shorter word, or part of a name like base64 or hex",
                ))
            })
    }

    fn category_header(
        &self,
        name: &'static str,
        ported: usize,
        total: Option<usize>,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let open = self.expanded.contains(name);
        let count = match total {
            Some(t) if ported < t => format!("{ported}/{t}"),
            _ => ported.to_string(),
        };
        let available = ported > 0 || total.is_none();
        h_flex()
            .id(SharedString::from(format!("cat-{name}")))
            .h(px(34.))
            .px(px(8.))
            .gap(px(8.))
            .rounded(radius(R_SM))
            .cursor_pointer()
            .hover(|s| s.bg(hover_tint()))
            .active(|s| s.bg(press_tint()))
            .child(ui::icon(
                if open {
                    "chevron-down"
                } else {
                    "chevron-right"
                },
                13.,
                fg3(),
            ))
            .child(
                ui::ui_text(text::SMALL, if available { fg() } else { fg3() })
                    .flex_1()
                    .min_w_0()
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .font_weight(FontWeight::MEDIUM)
                    .child(name),
            )
            .child(ui::mono(text::MICRO, fg3()).child(count))
            .on_click(cx.listener(move |this, _, _, cx| {
                if !this.expanded.remove(name) {
                    this.expanded.insert(name);
                }
                cx.notify();
            }))
    }

    fn op_row(
        &self,
        section: &str,
        op: &'static Operation,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let favourite = self.session.is_favourite(op.name);
        let hovered = self.hovered.is_some_and(|h| std::ptr::eq(h, op));
        h_flex()
            .id(SharedString::from(format!("op-{section}-{}", op.name)))
            .h(px(32.))
            .pl(px(29.))
            .pr(px(4.))
            .gap(px(6.))
            .rounded(radius(R_SM))
            .text_size(px(text::SMALL))
            .text_color(fg2())
            .cursor_pointer()
            .hover(|s| s.bg(hover_tint()).text_color(fg()))
            .active(|s| s.bg(press_tint()))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .child(op.name),
            )
            .child(
                div()
                    .id(SharedString::from(format!("fav-{section}-{}", op.name)))
                    .flex_none()
                    .size(px(28.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(radius(R_SM))
                    .hover(|s| s.bg(hover_tint()))
                    .active(|s| s.bg(press_tint()))
                    .tooltip(ui::tip(
                        if favourite {
                            "Remove from favourites"
                        } else {
                            "Add to favourites"
                        },
                        None,
                    ))
                    .when(favourite || hovered, |d| {
                        d.child(ui::icon(
                            "star",
                            14.,
                            if favourite { caution() } else { fg4() },
                        ))
                    })
                    .on_click(cx.listener(move |this, _, _, cx| {
                        cx.stop_propagation();
                        this.session.toggle_favourite(op.name);
                        this.save_soon(cx);
                        cx.notify();
                    })),
            )
            .on_hover(cx.listener(move |this, over: &bool, _, cx| {
                if *over {
                    this.hovered = Some(op);
                } else if this.hovered.is_some_and(|h| std::ptr::eq(h, op)) {
                    this.hovered = None;
                }
                cx.notify();
            }))
            .on_click(cx.listener(move |this, _, window, cx| this.add_op(op, None, window, cx)))
            .on_drag(DraggedOp(op), |d, _, _, cx| {
                cx.new(|_| DragLabel(d.0.name.into()))
            })
    }

    /// The hovered operation's description, or how to use the list.
    fn description_box(&self) -> Div {
        let (title, body): (SharedString, SharedString) = match self.hovered {
            Some(op) => (op.name.into(), op.description.into()),
            None => (
                "Add operations".into(),
                "Click an operation to add it to the end of the recipe, or drag it into place. Star the ones you use most.".into(),
            ),
        };
        v_flex()
            .flex_none()
            .h(px(140.))
            .mx(px(12.))
            .mb(px(12.))
            .p(px(13.))
            .gap(px(6.))
            .rounded(radius(R_MD))
            .bg(control())
            .border_1()
            .border_color(control_border())
            .overflow_hidden()
            .child(
                ui::ui_text(text::SMALL, fg())
                    .font_weight(FontWeight::MEDIUM)
                    .child(title),
            )
            .child(
                ui::ui_text(text::CAPTION, fg2())
                    .line_height(px(20.))
                    .child(body),
            )
    }
}

fn unported_row(name: &str) -> Div {
    h_flex()
        .h(px(32.))
        .pl(px(29.))
        .pr(px(8.))
        .gap(px(8.))
        .text_size(px(text::SMALL))
        .text_color(fg3())
        .child(
            div()
                .flex_1()
                .min_w_0()
                .overflow_hidden()
                .whitespace_nowrap()
                .child(name.to_string()),
        )
        .child(chip("soon", fg3()))
}
