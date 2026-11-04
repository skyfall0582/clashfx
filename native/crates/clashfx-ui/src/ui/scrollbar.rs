use super::*;

fn geometry(handle: &ScrollHandle) -> (f32, f32, f32) {
    let height = f32::from(handle.bounds().size.height).max(1.);
    let maximum = f32::from(handle.max_offset().height).max(0.);
    let thumb = (height * height / (height + maximum)).max(28.).min(height);
    (height, maximum, thumb)
}

impl ClashFx {
    pub(super) fn log_scrollbar(&self, cx: &mut Context<Self>) -> AnyElement {
        let handle = self.log_scroll.clone();
        div()
            .id("log-scrollbar")
            .absolute()
            .top_0()
            .bottom_0()
            .right_0()
            .w(px(14.))
            .cursor_pointer()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, event: &MouseDownEvent, _, cx| {
                    let (height, maximum, thumb) = geometry(&this.log_scroll);
                    if maximum <= 0. {
                        return;
                    }
                    let y = f32::from(event.position.y - this.log_scroll.bounds().origin.y);
                    let offset = -f32::from(this.log_scroll.offset().y);
                    let top = offset / maximum * (height - thumb);
                    if y < top || y > top + thumb {
                        let offset =
                            ((y - thumb / 2.) / (height - thumb) * maximum).clamp(0., maximum);
                        this.log_scroll.set_offset(point(px(0.), px(-offset)));
                    }
                    this.log_scroll_drag = Some((event.position.y, this.log_scroll.offset().y));
                    cx.stop_propagation();
                    cx.notify();
                }),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    this.log_scroll_drag = None;
                    cx.notify();
                }),
            )
            .child(
                canvas(
                    move |_, _, _| {
                        let (height, maximum, thumb) = geometry(&handle);
                        let top = if maximum > 0. {
                            -f32::from(handle.offset().y) / maximum * (height - thumb)
                        } else {
                            0.
                        };
                        (top, thumb, maximum)
                    },
                    |bounds, (top, thumb, maximum), window, _| {
                        let track = Bounds::new(
                            point(bounds.origin.x + px(4.), bounds.origin.y + px(3.)),
                            size(px(6.), bounds.size.height - px(6.)),
                        );
                        window.paint_quad(quad(
                            track,
                            px(3.),
                            rgb(0xf1ede8),
                            px(0.),
                            transparent_black(),
                            BorderStyle::Solid,
                        ));
                        if maximum > 0. {
                            let thumb_bounds = Bounds::new(
                                point(track.origin.x, bounds.origin.y + px(top.max(3.))),
                                size(px(6.), px((thumb - 6.).max(22.))),
                            );
                            window.paint_quad(quad(
                                thumb_bounds,
                                px(3.),
                                rgb(0xb8aa9d),
                                px(0.),
                                transparent_black(),
                                BorderStyle::Solid,
                            ));
                        }
                    },
                )
                .size_full(),
            )
            .into_any_element()
    }

    pub(super) fn drag_log_scrollbar(
        &mut self,
        event: &MouseMoveEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some((origin, offset)) = self.log_scroll_drag {
            let (height, maximum, thumb) = geometry(&self.log_scroll);
            if height > thumb {
                let y = f32::from(offset)
                    - f32::from(event.position.y - origin) / (height - thumb) * maximum;
                self.log_scroll
                    .set_offset(point(px(0.), px(y.clamp(-maximum, 0.))));
                cx.notify();
            }
        }
    }
}
