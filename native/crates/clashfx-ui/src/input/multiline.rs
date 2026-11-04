use super::*;
use gpui::{TextAlign, WrappedLine};

#[derive(Clone)]
pub(super) struct MultilineLayout {
    lines: Vec<WrappedLine>,
    pub(super) line_height: Pixels,
}

impl MultilineLayout {
    pub(super) fn height(&self) -> Pixels {
        self.lines
            .iter()
            .map(|line| line.size(self.line_height).height)
            .fold(px(0.), |height, line| height + line)
    }

    pub(super) fn position(&self, index: usize) -> Point<Pixels> {
        let mut start = 0;
        let mut y = px(0.);
        for line in &self.lines {
            if index <= start + line.len() {
                return line
                    .position_for_index(index - start, self.line_height)
                    .unwrap_or_default()
                    + point(px(0.), y);
            }
            start += line.len() + 1;
            y += line.size(self.line_height).height;
        }
        point(px(0.), (y - self.line_height).max(px(0.)))
    }

    pub(super) fn index(&self, position: Point<Pixels>) -> usize {
        if position.y < px(0.) {
            return 0;
        }
        let mut start = 0;
        let mut y = px(0.);
        for line in &self.lines {
            let height = line.size(self.line_height).height;
            if position.y < y + height {
                let index = line
                    .closest_index_for_position(position - point(px(0.), y), self.line_height)
                    .unwrap_or_else(|index| index);
                return start + index;
            }
            start += line.len() + 1;
            y += height;
        }
        start.saturating_sub(1)
    }

    fn selection(&self, range: Range<usize>, origin: Point<Pixels>) -> Vec<PaintQuad> {
        let mut quads = Vec::new();
        let mut start = 0;
        let mut y = px(0.);
        for (line_index, line) in self.lines.iter().enumerate() {
            let mut row_start = 0;
            let ends = line
                .wrap_boundaries
                .iter()
                .map(|boundary| {
                    line.unwrapped_layout.runs[boundary.run_ix].glyphs[boundary.glyph_ix].index
                })
                .chain([line.len()]);
            for row_end in ends {
                let from = range.start.max(start + row_start);
                let to = range.end.min(start + row_end);
                let newline = row_end == line.len()
                    && line_index + 1 < self.lines.len()
                    && range.start <= start + row_end
                    && range.end > start + row_end;
                if from < to || newline {
                    let row_x = line.unwrapped_layout.x_for_index(row_start);
                    let left = line
                        .unwrapped_layout
                        .x_for_index(range.start.saturating_sub(start).clamp(row_start, row_end))
                        - row_x;
                    let mut right = line
                        .unwrapped_layout
                        .x_for_index(range.end.saturating_sub(start).clamp(row_start, row_end))
                        - row_x;
                    if newline {
                        right += px(5.);
                    }
                    quads.push(fill(
                        Bounds::new(
                            origin + point(left, y),
                            size((right - left).max(px(1.)), self.line_height),
                        ),
                        rgba((crate::ui::ACCENT << 8) | 0x30),
                    ));
                }
                row_start = row_end;
                y += self.line_height;
            }
            start += line.len() + 1;
        }
        quads
    }
}

pub(super) struct MultilineElement {
    pub(super) input: Entity<TextInput>,
}

pub(super) struct MultilinePrepaint {
    layout: Option<MultilineLayout>,
    scroll_y: Pixels,
    selection: Vec<PaintQuad>,
    cursor: Option<PaintQuad>,
}

impl IntoElement for MultilineElement {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for MultilineElement {
    type RequestLayoutState = ();
    type PrepaintState = MultilinePrepaint;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        let mut style = Style::default();
        style.size.width = relative(1.).into();
        style.size.height = relative(1.).into();
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) -> MultilinePrepaint {
        let input = self.input.read(cx);
        let style = window.text_style();
        let (text, color) = if input.content.is_empty() {
            (input.placeholder.clone(), hsla(0., 0., 0., 0.2))
        } else {
            (input.content.clone(), style.color)
        };
        let run = TextRun {
            len: text.len(),
            font: style.font(),
            color,
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        let runs = if let Some(marked) = &input.marked_range {
            vec![
                TextRun {
                    len: marked.start,
                    ..run.clone()
                },
                TextRun {
                    len: marked.end - marked.start,
                    underline: Some(UnderlineStyle {
                        color: Some(color),
                        thickness: px(1.),
                        wavy: false,
                    }),
                    ..run.clone()
                },
                TextRun {
                    len: text.len() - marked.end,
                    ..run
                },
            ]
            .into_iter()
            .filter(|run| run.len > 0)
            .collect()
        } else {
            vec![run]
        };
        let lines = window
            .text_system()
            .shape_text(
                text,
                style.font_size.to_pixels(window.rem_size()),
                &runs,
                Some((bounds.size.width - px(8.)).max(px(1.))),
                None,
            )
            .expect("shape multiline input")
            .into_iter()
            .collect();
        let layout = MultilineLayout {
            lines,
            line_height: window.line_height(),
        };
        let caret = layout.position(input.cursor_offset());
        let maximum = (layout.height() - bounds.size.height).max(px(0.));
        let mut scroll_y = input.scroll_y.clamp(px(0.), maximum);
        if input.scroll_to_cursor && input.focus_handle.is_focused(window) {
            if caret.y < scroll_y {
                scroll_y = caret.y;
            } else if caret.y + layout.line_height > scroll_y + bounds.size.height {
                scroll_y = caret.y + layout.line_height - bounds.size.height;
            }
        }
        let origin = bounds.origin - point(px(0.), scroll_y);
        let selection = layout.selection(input.selected_range.clone(), origin);
        let cursor = input.selected_range.is_empty().then(|| {
            fill(
                Bounds::new(origin + caret, size(px(2.), layout.line_height)),
                rgb(crate::ui::ACCENT),
            )
        });
        MultilinePrepaint {
            layout: Some(layout),
            scroll_y,
            selection,
            cursor,
        }
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        prepaint: &mut MultilinePrepaint,
        window: &mut Window,
        cx: &mut App,
    ) {
        let focus = self.input.read(cx).focus_handle.clone();
        window.handle_input(
            &focus,
            ElementInputHandler::new(bounds, self.input.clone()),
            cx,
        );
        let layout = prepaint.layout.take().unwrap();
        window.with_content_mask(Some(gpui::ContentMask { bounds }), |window| {
            for selection in prepaint.selection.drain(..) {
                window.paint_quad(selection);
            }
            let mut origin = bounds.origin - point(px(0.), prepaint.scroll_y);
            for line in &layout.lines {
                line.paint(
                    origin,
                    layout.line_height,
                    TextAlign::Left,
                    None,
                    window,
                    cx,
                )
                .unwrap();
                origin.y += line.size(layout.line_height).height;
            }
            if focus.is_focused(window)
                && let Some(cursor) = prepaint.cursor.take()
            {
                window.paint_quad(cursor);
            }
            if layout.height() > bounds.size.height {
                let thumb =
                    (bounds.size.height * (bounds.size.height / layout.height())).max(px(16.));
                let top = (bounds.size.height - thumb)
                    * (prepaint.scroll_y / (layout.height() - bounds.size.height));
                window.paint_quad(fill(
                    Bounds::new(
                        point(bounds.right() - px(3.), bounds.top() + top),
                        size(px(3.), thumb),
                    ),
                    rgb(0xb8aa9d),
                ));
            }
        });
        self.input.update(cx, |input, _| {
            input.last_multiline = Some(layout);
            input.last_bounds = Some(bounds);
            input.scroll_y = prepaint.scroll_y;
            input.scroll_to_cursor = false;
        });
    }
}
