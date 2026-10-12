use crate::{
    emit,
    text::{byte_range, to_utf16, utf16_range, Text},
};
use gpui::{prelude::*, *};
use live_gpui_protocol::Node;
use serde_json::{json, Value};
use std::ops::Range;

actions!(
    live_input,
    [
        Backspace,
        Delete,
        Left,
        Right,
        SelectLeft,
        SelectRight,
        SelectAll,
        Home,
        End,
        Copy,
        Cut,
        Paste
    ]
);

pub struct NativeNode {
    pub node: Node,
    pub children: Vec<Entity<NativeNode>>,
    pub text: Text,
    pub focus: FocusHandle,
    pub generation: u64,
    pub application_revision: u64,
    element_id: SharedString,
    label: SharedString,
    placeholder: SharedString,
    display: SharedString,
    display_placeholder: bool,
    scroll_x: f32,
    line: Option<ShapedLine>,
    bounds: Option<Bounds<Pixels>>,
    selecting: bool,
}
impl NativeNode {
    pub fn new(
        node: Node,
        generation: u64,
        application_revision: u64,
        cx: &mut Context<Self>,
    ) -> Self {
        let value = string(&node, "value").to_owned();
        let revision = node
            .props
            .get("edit_revision")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        let element_id = node.id.clone().into();
        let label = string(&node, "text").to_owned().into();
        let placeholder: SharedString = string(&node, "placeholder").to_owned().into();
        let display_placeholder = value.is_empty();
        let display = if value.is_empty() {
            placeholder.clone()
        } else {
            value.clone().into()
        };
        Self {
            node,
            children: vec![],
            text: Text::new(value, revision),
            focus: cx.focus_handle(),
            generation,
            application_revision,
            element_id,
            label,
            placeholder,
            display,
            display_placeholder,
            scroll_x: 0.,
            line: None,
            bounds: None,
            selecting: false,
        }
    }
    pub fn update_properties(&mut self) {
        let label = string(&self.node, "text");
        if self.label.as_ref() != label {
            self.label = label.to_owned().into();
        }
        let placeholder = string(&self.node, "placeholder");
        if self.placeholder.as_ref() != placeholder {
            self.placeholder = placeholder.to_owned().into();
        }
    }
    pub fn refresh_display(&mut self) -> bool {
        let placeholder = self.text.value.is_empty();
        let value = if placeholder {
            self.placeholder.as_ref()
        } else {
            self.text.value.as_str()
        };
        let value_changed = self.display.as_ref() != value;
        if !value_changed && self.display_placeholder == placeholder {
            return false;
        }
        if value_changed {
            self.display = value.to_owned().into();
        }
        self.display_placeholder = placeholder;
        self.line = None;
        true
    }
    pub fn event(&self, property: &str) {
        if let Some(name) = self.node.props.get(property).and_then(Value::as_str) {
            let mut params = serde_json::Map::new();
            for (k, v) in &self.node.props {
                if let Some(k) = k.strip_prefix("phx-value-") {
                    params.insert(k.to_owned(), v.clone());
                }
            }
            if property == "phx-change" {
                params.insert("value".into(), json!(self.text.value));
                params.insert("edit_revision".into(), json!(self.text.revision));
            }
            emit(
                json!({"type":"event","generation":self.generation,"revision":self.application_revision,"node":self.node.id,"name":name,"params":params}),
            );
        }
    }
    fn changed(&mut self, cx: &mut Context<Self>) {
        self.refresh_display();
        self.event("phx-change");
        cx.notify();
    }
    fn move_caret(&mut self, at: usize, extend: bool, cx: &mut Context<Self>) {
        self.text.move_to(at, extend);
        cx.notify();
    }
    fn left(&mut self, _: &Left, _: &mut Window, cx: &mut Context<Self>) {
        let at = if self.text.selection.is_empty() {
            self.text.previous()
        } else {
            self.text.selection.start
        };
        self.move_caret(at, false, cx);
    }
    fn right(&mut self, _: &Right, _: &mut Window, cx: &mut Context<Self>) {
        let at = if self.text.selection.is_empty() {
            self.text.next()
        } else {
            self.text.selection.end
        };
        self.move_caret(at, false, cx);
    }
    fn select_left(&mut self, _: &SelectLeft, _: &mut Window, cx: &mut Context<Self>) {
        self.move_caret(self.text.previous(), true, cx);
    }
    fn select_right(&mut self, _: &SelectRight, _: &mut Window, cx: &mut Context<Self>) {
        self.move_caret(self.text.next(), true, cx);
    }
    fn select_all(&mut self, _: &SelectAll, _: &mut Window, cx: &mut Context<Self>) {
        self.text.move_to(0, false);
        self.move_caret(self.text.value.len(), true, cx);
    }
    fn home(&mut self, _: &Home, _: &mut Window, cx: &mut Context<Self>) {
        self.move_caret(0, false, cx);
    }
    fn end(&mut self, _: &End, _: &mut Window, cx: &mut Context<Self>) {
        self.move_caret(self.text.value.len(), false, cx);
    }
    fn backspace(&mut self, _: &Backspace, _: &mut Window, cx: &mut Context<Self>) {
        if self.text.selection.is_empty() {
            self.text.move_to(self.text.previous(), true);
        }
        self.text.replace(None, "", false, None);
        self.changed(cx);
    }
    fn delete(&mut self, _: &Delete, _: &mut Window, cx: &mut Context<Self>) {
        if self.text.selection.is_empty() {
            self.text.move_to(self.text.next(), true);
        }
        self.text.replace(None, "", false, None);
        self.changed(cx);
    }
    fn copy(&mut self, _: &Copy, _: &mut Window, cx: &mut Context<Self>) {
        if !self.text.selection.is_empty() {
            cx.write_to_clipboard(ClipboardItem::new_string(
                self.text.value[self.text.selection.clone()].to_owned(),
            ));
        }
    }
    fn cut(&mut self, _: &Cut, window: &mut Window, cx: &mut Context<Self>) {
        if !self.text.selection.is_empty() {
            self.copy(&Copy, window, cx);
            self.text.replace(None, "", false, None);
            self.changed(cx);
        }
    }
    fn paste(&mut self, _: &Paste, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
            self.text.replace(None, &text, false, None);
            self.changed(cx);
        }
    }
    fn mouse_index(&self, point: Point<Pixels>) -> usize {
        match (&self.line, self.bounds) {
            (Some(line), Some(bounds)) if !self.text.value.is_empty() => line
                .closest_index_for_x(point.x - bounds.left() + px(self.scroll_x))
                .min(self.text.value.len()),
            _ => 0,
        }
    }
    fn mouse_down(&mut self, event: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus);
        self.selecting = true;
        self.move_caret(self.mouse_index(event.position), event.modifiers.shift, cx);
    }
    fn mouse_move(&mut self, event: &MouseMoveEvent, _: &mut Window, cx: &mut Context<Self>) {
        if self.selecting {
            self.move_caret(self.mouse_index(event.position), true, cx);
        }
    }
    fn mouse_up(&mut self, _: &MouseUpEvent, _: &mut Window, _: &mut Context<Self>) {
        self.selecting = false;
    }
}
pub fn string<'a>(node: &'a Node, key: &str) -> &'a str {
    node.props.get(key).and_then(Value::as_str).unwrap_or("")
}
impl EntityInputHandler for NativeNode {
    fn text_for_range(
        &mut self,
        range: Range<usize>,
        actual: &mut Option<Range<usize>>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<String> {
        let range = byte_range(&self.text.value, &range);
        *actual = Some(utf16_range(&self.text.value, &range));
        Some(self.text.value[range].to_owned())
    }
    fn selected_text_range(
        &mut self,
        _: bool,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        Some(UTF16Selection {
            range: utf16_range(&self.text.value, &self.text.selection),
            reversed: self.text.reversed,
        })
    }
    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> {
        self.text
            .marked
            .as_ref()
            .map(|r| utf16_range(&self.text.value, r))
    }
    fn unmark_text(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        self.text.unmark();
        self.refresh_display();
        cx.notify();
    }
    fn replace_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        value: &str,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.text.replace(range, value, false, None);
        self.changed(cx);
    }
    fn replace_and_mark_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        value: &str,
        selected: Option<Range<usize>>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.text.replace(range, value, true, selected);
        self.changed(cx);
    }
    fn bounds_for_range(
        &mut self,
        range: Range<usize>,
        bounds: Bounds<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let line = self.line.as_ref()?;
        let r = byte_range(&self.text.value, &range);
        Some(Bounds::from_corners(
            point(
                bounds.left() + line.x_for_index(r.start) - px(self.scroll_x),
                bounds.top(),
            ),
            point(
                bounds.left() + line.x_for_index(r.end) - px(self.scroll_x),
                bounds.bottom(),
            ),
        ))
    }
    fn character_index_for_point(
        &mut self,
        position: Point<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<usize> {
        let bounds = self.bounds?;
        let index = self
            .line
            .as_ref()?
            .closest_index_for_x(position.x - bounds.left() + px(self.scroll_x))
            .min(self.text.value.len());
        Some(to_utf16(&self.text.value, index))
    }
}
impl Render for NativeNode {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let base = div().id(self.element_id.clone()).flex().gap(px(self
            .node
            .props
            .get("gap")
            .and_then(Value::as_u64)
            .unwrap_or(8)
            as f32));
        match self.node.kind.as_str() {
            "row" => base
                .flex_row()
                .w_full()
                .min_w_0()
                .flex_wrap()
                .children(self.children.iter().cloned())
                .into_any_element(),
            "column" | "list" => base
                .flex_col()
                .w_full()
                .min_w_0()
                .children(self.children.iter().cloned())
                .into_any_element(),
            "text" => base
                .block()
                .min_w_0()
                .whitespace_normal()
                .child(self.label.clone())
                .into_any_element(),
            "button" => base
                .block()
                .min_w_0()
                .max_w_full()
                .flex_shrink_0()
                .whitespace_normal()
                .p_2()
                .bg(rgb(0x365a91))
                .text_color(rgb(0xffffff))
                .cursor_pointer()
                .on_click(cx.listener(|this, _, _, _| this.event("phx-click")))
                .child(self.label.clone())
                .into_any_element(),
            "input" => base
                .flex_1()
                .h(px(46.))
                .max_h(px(46.))
                .min_w(px(180.))
                .overflow_hidden()
                .p_2()
                .bg(rgb(0xffffff))
                .border_1()
                .border_color(rgb(0x7c8795))
                .key_context("LiveInput")
                .track_focus(&self.focus)
                .cursor(CursorStyle::IBeam)
                .on_action(cx.listener(Self::left))
                .on_action(cx.listener(Self::right))
                .on_action(cx.listener(Self::select_left))
                .on_action(cx.listener(Self::select_right))
                .on_action(cx.listener(Self::select_all))
                .on_action(cx.listener(Self::home))
                .on_action(cx.listener(Self::end))
                .on_action(cx.listener(Self::backspace))
                .on_action(cx.listener(Self::delete))
                .on_action(cx.listener(Self::copy))
                .on_action(cx.listener(Self::cut))
                .on_action(cx.listener(Self::paste))
                .on_mouse_down(MouseButton::Left, cx.listener(Self::mouse_down))
                .on_mouse_move(cx.listener(Self::mouse_move))
                .on_mouse_up(MouseButton::Left, cx.listener(Self::mouse_up))
                .on_mouse_up_out(MouseButton::Left, cx.listener(Self::mouse_up))
                .child(InputSurface(cx.entity()))
                .into_any_element(),
            _ => unreachable!(),
        }
    }
}
struct InputSurface(Entity<NativeNode>);
struct PaintedText {
    line: ShapedLine,
    selection: Option<PaintQuad>,
    caret: Option<PaintQuad>,
    marked: Option<PaintQuad>,
    origin: Point<Pixels>,
    scroll_x: f32,
}
impl IntoElement for InputSurface {
    type Element = Self;
    fn into_element(self) -> Self {
        self
    }
}
impl Element for InputSurface {
    type RequestLayoutState = ();
    type PrepaintState = PaintedText;
    fn id(&self) -> Option<ElementId> {
        None
    }
    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }
    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        let mut style = Style::default();
        style.size.width = relative(1.).into();
        style.size.height = px(28.).into();
        (window.request_layout(style, [], cx), ())
    }
    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) -> PaintedText {
        let node = self.0.read(cx);
        let text = &node.text;
        let placeholder = text.value.is_empty();
        let line = node.line.clone().unwrap_or_else(|| {
            let display = node.display.clone();
            let style = window.text_style();
            let run = TextRun {
                len: display.len(),
                font: style.font(),
                color: if placeholder {
                    rgb(0x888888).into()
                } else {
                    rgb(0x18202a).into()
                },
                background_color: None,
                underline: None,
                strikethrough: None,
            };
            window
                .text_system()
                .shape_line(display, px(18.), &[run], None)
        });
        let visible_width = (f32::from(bounds.size.width) - 2.).max(0.);
        let caret_x = f32::from(line.x_for_index(text.caret()));
        let mut scroll_x = node
            .scroll_x
            .min((f32::from(line.width) - visible_width).max(0.));
        if caret_x < scroll_x {
            scroll_x = caret_x;
        }
        if caret_x > scroll_x + visible_width {
            scroll_x = caret_x - visible_width;
        }
        let origin = point(bounds.left() - px(scroll_x), bounds.top());
        let quad = |r: &Range<usize>, color| {
            fill(
                Bounds::from_corners(
                    point(origin.x + line.x_for_index(r.start), bounds.top()),
                    point(origin.x + line.x_for_index(r.end), bounds.bottom()),
                ),
                color,
            )
        };
        let selection =
            (!text.selection.is_empty()).then(|| quad(&text.selection, rgba(0x5588cc55)));
        let caret = (text.selection.is_empty() && node.focus.is_focused(window)).then(|| {
            fill(
                Bounds::new(
                    point(origin.x + line.x_for_index(text.caret()), bounds.top()),
                    size(px(1.5), bounds.size.height),
                ),
                rgb(0x18202a),
            )
        });
        let marked = text.marked.as_ref().map(|r| {
            fill(
                Bounds::new(
                    point(
                        origin.x + line.x_for_index(r.start),
                        bounds.bottom() - px(2.),
                    ),
                    size(line.x_for_index(r.end) - line.x_for_index(r.start), px(1.)),
                ),
                rgb(0x18202a),
            )
        });
        PaintedText {
            line,
            selection,
            caret,
            marked,
            origin,
            scroll_x,
        }
    }
    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        state: &mut PaintedText,
        window: &mut Window,
        cx: &mut App,
    ) {
        window.handle_input(
            &self.0.read(cx).focus.clone(),
            ElementInputHandler::new(bounds, self.0.clone()),
            cx,
        );
        window.with_content_mask(Some(ContentMask { bounds }), |window| {
            if let Some(q) = state.selection.take() {
                window.paint_quad(q);
            }
            if let Err(error) = state.line.paint(state.origin, px(28.), window, cx) {
                eprintln!("text painting: {error}");
            }
            if let Some(q) = state.caret.take() {
                window.paint_quad(q);
            }
            if let Some(q) = state.marked.take() {
                window.paint_quad(q);
            }
        });
        self.0.update(cx, |node, _| {
            node.line = Some(state.line.clone());
            node.bounds = Some(bounds);
            node.scroll_x = state.scroll_x;
        });
    }
}
