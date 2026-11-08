//! Pieces the workbench's panels are built from, in the dashboard's style
//! (see [`utilify_app::ui`]): inset panels, small icon and text buttons, a
//! dropdown and the frame text fields sit in.

use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use utilify_app::ui::{
    self, R_MD, R_SM,
    color::{self, *},
    px, radius, text,
};

/// Hover tint for flat controls. A faint wash of the text colour, so it
/// reads on any surface (panel, step card, popover) in either appearance.
pub fn hover_tint() -> Hsla {
    fg().opacity(0.07)
}

/// Pressed tint: a step darker (or lighter, in dark) than [`hover_tint`].
pub fn press_tint() -> Hsla {
    fg().opacity(0.13)
}

/// How much a disabled control fades.
const DISABLED: f32 = 0.45;

/// An inset panel with a title row (title, caption, actions) over its body.
pub fn panel(
    title: &str,
    caption: impl IntoElement,
    actions: impl IntoElement,
    body: impl IntoElement,
) -> Div {
    v_flex()
        .min_w_0()
        .min_h_0()
        .overflow_hidden()
        .bg(color::panel())
        .border_1()
        .border_color(panel_border())
        .rounded(radius(R_MD))
        .child(
            h_flex()
                .flex_none()
                .h(px(50.))
                .px(px(16.))
                .gap(px(10.))
                .border_b_1()
                .border_color(sep())
                .child(
                    ui::ui_text(text::BODY, fg())
                        .font_weight(FontWeight::MEDIUM)
                        .flex_none()
                        .child(title.to_string()),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_size(px(text::CAPTION))
                        .text_color(fg3())
                        .child(caption),
                )
                .child(h_flex().flex_none().gap(px(6.)).child(actions)),
        )
        .child(body)
}

/// A 30 px square button with an icon, for panel title rows and step cards.
/// Icons alone don't say what they do, so it always has a tooltip. When
/// disabled it fades and ignores the pointer (callers skip `on_click`).
pub fn tool_button(
    id: impl Into<ElementId>,
    icon: &str,
    color: Hsla,
    label: impl Into<SharedString>,
    enabled: bool,
) -> Stateful<Div> {
    tool_button_tinted(id, icon, color, label, enabled, hover_tint())
}

/// A [`tool_button`] for removing something: its hover warns in the
/// danger colour.
pub fn remove_button(id: impl Into<ElementId>, label: impl Into<SharedString>) -> Stateful<Div> {
    tool_button_tinted(id, "x", fg3(), label, true, danger().opacity(0.14))
}

fn tool_button_tinted(
    id: impl Into<ElementId>,
    icon: &str,
    color: Hsla,
    label: impl Into<SharedString>,
    enabled: bool,
    hover: Hsla,
) -> Stateful<Div> {
    div()
        .id(id)
        .flex_none()
        .size(px(30.))
        .rounded(radius(R_SM))
        .flex()
        .items_center()
        .justify_center()
        .tooltip(ui::tip(label, None))
        .when(enabled, |b| {
            b.cursor_pointer()
                .hover(move |s| s.bg(hover))
                .active(|s| s.bg(press_tint()))
        })
        .when(!enabled, |b| b.opacity(DISABLED))
        .child(ui::icon(icon, 15., color))
}

/// A small bordered text button: `Copy`, `Paste`…
pub fn text_button(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    enabled: bool,
) -> Stateful<Div> {
    div()
        .id(id)
        .flex_none()
        .h(px(30.))
        .px(px(11.))
        .flex()
        .items_center()
        .gap(px(6.))
        .rounded(radius(R_SM))
        .border_1()
        .border_color(raised_border())
        .bg(raised())
        .text_size(px(text::CAPTION))
        .text_color(fg2())
        .when(enabled, |b| {
            b.cursor_pointer()
                .hover(|s| s.bg(raised_hover()).text_color(fg()))
                .active(|s| s.bg(press_tint()).text_color(fg()))
        })
        .when(!enabled, |b| b.opacity(DISABLED))
        .child(label.into())
}

/// A small coloured label: `binary`, `paused`, `soon`.
pub fn chip(label: impl Into<SharedString>, color: Hsla) -> Div {
    ui::pill(label, color)
}

/// The line that shows where a dragged operation or step will land: drawn
/// in the focus colour while something hovers over `group`, else hidden.
/// Sits absolutely in the gap above its parent, which must be `relative`.
pub fn drop_line<T: 'static, U: 'static>(group: SharedString) -> Div {
    div()
        .absolute()
        .top(px(-7.))
        .left(px(4.))
        .right(px(4.))
        .h(px(3.))
        .rounded(radius(R_SM))
        // Group drag styles only apply to elements with a hitbox, and a
        // hover style is the cheapest way to get one.
        .hover(|s| s)
        .group_drag_over::<T>(group.clone(), |s| s.bg(focus()))
        .group_drag_over::<U>(group, |s| s.bg(focus()))
}

/// What picking a dropdown entry does.
pub type PickHandler = Box<dyn Fn(&ClickEvent, &mut Window, &mut App)>;

/// A dropdown entry.
pub struct Choice {
    pub label: SharedString,
    pub selected: bool,
    pub on_pick: PickHandler,
}

/// A pill showing `current`; when `open`, a menu of `choices` under it.
/// `on_toggle` flips it open, `on_dismiss` closes it on a click elsewhere.
pub fn dropdown(
    id: SharedString,
    current: impl Into<SharedString>,
    open: bool,
    choices: Vec<Choice>,
    on_toggle: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    on_dismiss: impl Fn(&MouseDownEvent, &mut Window, &mut App) + 'static,
) -> Div {
    let pill = h_flex()
        .id(id.clone())
        .h(px(36.))
        .w_full()
        .px(px(11.))
        .gap(px(8.))
        .justify_between()
        .rounded(radius(R_MD))
        .bg(color::panel())
        .border_1()
        .border_color(if open { focus() } else { control_border() })
        .text_size(px(text::SMALL))
        .text_color(fg())
        .cursor_pointer()
        .hover(|s| s.border_color(if open { focus() } else { hairline() }))
        .active(|s| s.bg(control_hover()))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .overflow_hidden()
                .whitespace_nowrap()
                .child(current.into()),
        )
        .child(ui::icon(
            if open { "chevron-up" } else { "chevron-down" },
            13.,
            fg3(),
        ))
        .on_click(on_toggle);

    let menu = open.then(|| {
        let items = choices.into_iter().enumerate().map(|(i, c)| {
            h_flex()
                .id(i)
                .min_h(px(32.))
                .px(px(10.))
                .gap(px(8.))
                .rounded(radius(R_SM))
                .text_size(px(text::SMALL))
                .text_color(if c.selected { fg() } else { fg2() })
                .when(c.selected, |r| r.font_weight(FontWeight::MEDIUM))
                .cursor_pointer()
                .hover(|s| s.bg(hover_tint()).text_color(fg()))
                .active(|s| s.bg(press_tint()))
                .child(div().flex_1().min_w_0().child(c.label))
                .child(
                    div()
                        .w(px(14.))
                        .flex_none()
                        .children(c.selected.then(|| ui::icon("check", 14., ok()))),
                )
                .on_click(c.on_pick)
        });
        deferred(
            v_flex()
                .id(SharedString::from(format!("{id}-menu")))
                .absolute()
                .top(px(40.))
                .left_0()
                .right_0()
                .min_w(px(180.))
                .max_h(px(420.))
                .overflow_y_scroll()
                .p(px(5.))
                .bg(popover())
                .border_1()
                .border_color(raised_border())
                .rounded(radius(R_MD))
                .shadow(vec![BoxShadow {
                    color: shadow(),
                    offset: point(px(0.), px(10.)),
                    blur_radius: px(28.),
                    spread_radius: px(0.),
                    inset: false,
                }])
                .occlude()
                .children(items),
        )
        .with_priority(1)
    });

    div()
        .relative()
        .w_full()
        .child(pill)
        .children(menu)
        .when(open, |d| d.on_mouse_down_out(on_dismiss))
}

/// The frame a text field sits in, matching the dropdown pill. `focused`
/// draws the focus ring.
pub fn field_frame(focused: bool) -> Div {
    div()
        .w_full()
        .min_h(px(36.))
        .px(px(4.))
        .flex()
        .items_center()
        .rounded(radius(R_MD))
        .bg(control())
        .border_1()
        .border_color(if focused { focus() } else { control_border() })
        .font_family(utilify_app::theme::MONO_FONT)
        .text_size(px(text::SMALL))
        .text_color(fg())
}
