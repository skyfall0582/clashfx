use super::{ACCENT, INK};
use clashfx_core::model::{RoutingMode, Snapshot};
use gpui::{prelude::*, *};
use std::{sync::Arc, time::Duration};

const DIAGRAM_SCALE: f32 = 0.82;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum NetworkState {
    Off,
    Direct,
    Preparing,
    Online,
    Interrupted,
}

pub(crate) struct NetworkView {
    state: NetworkState,
    logo: Arc<Image>,
}

impl NetworkView {
    pub(crate) fn new(state: NetworkState, logo: Arc<Image>) -> Self {
        Self { state, logo }
    }

    pub(crate) fn set_state(&mut self, state: NetworkState, cx: &mut Context<Self>) {
        if self.state != state {
            self.state = state;
            cx.notify();
        }
    }
}

impl Render for NetworkView {
    fn render(&mut self, window: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        graphic(self.state, self.logo.clone(), window_can_animate(window))
    }
}

impl NetworkState {
    pub(crate) fn from_snapshot(snapshot: &Snapshot) -> Self {
        if !snapshot.system_proxy {
            return if !snapshot.running && snapshot.busy && !snapshot.nodes.is_empty() {
                Self::Preparing
            } else {
                Self::Off
            };
        }
        if snapshot.running && snapshot.config.routing_mode == RoutingMode::Direct {
            return Self::Direct;
        }
        if !snapshot.running || snapshot.active.is_none() || snapshot.phase == "池内节点全部不可用"
        {
            Self::Interrupted
        } else if snapshot.probing && snapshot.failure_streak > 0 {
            Self::Preparing
        } else {
            Self::Online
        }
    }

    fn description(self) -> &'static str {
        match self {
            Self::Off => "系统代理未开启",
            Self::Direct => "直连模式：流量不经过代理节点",
            Self::Preparing => "正在准备或检测代理连接",
            Self::Online => "系统代理运行中；流动表示运行状态，不表示流量大小",
            Self::Interrupted => "代理连接异常；请检查节点或查看运行日志",
        }
    }
}

pub(crate) fn window_can_animate(window: &Window) -> bool {
    #[cfg(windows)]
    {
        use raw_window_handle::{HasWindowHandle, RawWindowHandle};
        use windows_sys::Win32::UI::WindowsAndMessaging::{IsIconic, IsWindowVisible};
        if let Ok(handle) = HasWindowHandle::window_handle(window)
            && let RawWindowHandle::Win32(handle) = handle.as_raw()
        {
            let handle = handle.hwnd.get() as _;
            return unsafe { IsIconic(handle) == 0 && IsWindowVisible(handle) != 0 };
        }
    }
    #[cfg(not(windows))]
    let _ = window;
    true
}

pub(crate) fn graphic(state: NetworkState, logo: Arc<Image>, animate: bool) -> AnyElement {
    let diagram = NetworkDiagram {
        state,
        logo,
        phase: 0.,
    };
    let content = if animate && matches!(state, NetworkState::Online | NetworkState::Preparing) {
        diagram
            .with_animation(
                ("network-flow", state as usize),
                Animation::new(Duration::from_millis(3200)).repeat(),
                |mut diagram, phase| {
                    diagram.phase = phase;
                    diagram
                },
            )
            .into_any_element()
    } else {
        diagram.into_any_element()
    };
    div()
        .id("network-state")
        .flex_shrink_0()
        .tooltip(move |_, cx| cx.new(|_| VisualTip(state.description().to_owned())).into())
        .child(content)
        .into_any_element()
}

pub(super) fn status_icon(
    id: impl Into<ElementId>,
    path: &'static str,
    text: &str,
    color: u32,
) -> AnyElement {
    let text = text.to_owned();
    div()
        .id(id)
        .flex_shrink_0()
        .size(px(22.))
        .flex()
        .items_center()
        .justify_center()
        .tooltip(move |_, cx| cx.new(|_| VisualTip(text.clone())).into())
        .child(svg().path(path).size(px(17.)).text_color(rgb(color)))
        .into_any_element()
}

struct VisualTip(String);
impl Render for VisualTip {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .max_w(px(300.))
            .px_3()
            .py_2()
            .rounded_md()
            .bg(rgb(INK))
            .text_color(rgb(0xffffff))
            .text_size(px(12.))
            .child(self.0.clone())
    }
}

#[derive(IntoElement)]
struct NetworkDiagram {
    state: NetworkState,
    logo: Arc<Image>,
    phase: f32,
}

impl RenderOnce for NetworkDiagram {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        let state = self.state;
        let phase = self.phase;
        div()
            .relative()
            .w(px(320. * DIAGRAM_SCALE))
            .h(px(190. * DIAGRAM_SCALE))
            .child(
                canvas(
                    |_, _, _| (),
                    move |bounds, _, window, _| {
                        paint_network(bounds, window, state, phase);
                    },
                )
                .size_full(),
            )
            .child(
                div()
                    .absolute()
                    .left(px(138. * DIAGRAM_SCALE))
                    .top(px(73. * DIAGRAM_SCALE))
                    .size(px(44. * DIAGRAM_SCALE))
                    .rounded_lg()
                    .overflow_hidden()
                    .opacity(
                        if matches!(state, NetworkState::Off | NetworkState::Direct) {
                            0.35
                        } else {
                            1.
                        },
                    )
                    .child(img(self.logo).size_full()),
            )
    }
}

type Curve = [[f32; 2]; 4];
const ROUTES: [Curve; 4] = [
    [[57., 86.], [80., 39.], [105., 39.], [128., 86.]],
    [[192., 86.], [215., 39.], [240., 39.], [263., 86.]],
    [[263., 104.], [240., 151.], [215., 151.], [192., 104.]],
    [[128., 104.], [105., 151.], [80., 151.], [57., 104.]],
];

fn curve_point(curve: Curve, t: f32) -> [f32; 2] {
    let u = 1. - t;
    std::array::from_fn(|axis| {
        u.powi(3) * curve[0][axis]
            + 3. * u.powi(2) * t * curve[1][axis]
            + 3. * u * t.powi(2) * curve[2][axis]
            + t.powi(3) * curve[3][axis]
    })
}

struct Plot<'a> {
    origin: Point<Pixels>,
    window: &'a mut Window,
}

impl Plot<'_> {
    fn point(&self, xy: [f32; 2]) -> Point<Pixels> {
        self.origin + point(px(xy[0] * DIAGRAM_SCALE), px(xy[1] * DIAGRAM_SCALE))
    }

    fn line(&mut self, points: &[[f32; 2]], color: Rgba, width: f32) {
        let mut path = PathBuilder::stroke(px(width * DIAGRAM_SCALE));
        for (index, xy) in points.iter().enumerate() {
            if index == 0 {
                path.move_to(self.point(*xy));
            } else {
                path.line_to(self.point(*xy));
            }
        }
        if let Ok(path) = path.build() {
            self.window.paint_path(path, color);
        }
    }

    fn curve(&mut self, curve: Curve, color: Rgba, width: f32, dashed: bool) {
        let mut path = PathBuilder::stroke(px(width * DIAGRAM_SCALE));
        if dashed {
            path = path.dash_array(&[px(3. * DIAGRAM_SCALE), px(5. * DIAGRAM_SCALE)]);
        }
        path.move_to(self.point(curve[0]));
        path.cubic_bezier_to(
            self.point(curve[3]),
            self.point(curve[1]),
            self.point(curve[2]),
        );
        if let Ok(path) = path.build() {
            self.window.paint_path(path, color);
        }
    }

    fn circle(&mut self, center: [f32; 2], radius: f32, fill: Rgba, stroke: Rgba, width: f32) {
        self.rect(
            [center[0] - radius, center[1] - radius],
            [radius * 2., radius * 2.],
            radius,
            fill,
            stroke,
            width,
        );
    }

    fn rect(
        &mut self,
        top_left: [f32; 2],
        dimensions: [f32; 2],
        radius: f32,
        fill: Rgba,
        stroke: Rgba,
        width: f32,
    ) {
        self.window.paint_quad(quad(
            Bounds::new(
                self.point(top_left),
                size(
                    px(dimensions[0] * DIAGRAM_SCALE),
                    px(dimensions[1] * DIAGRAM_SCALE),
                ),
            ),
            px(radius * DIAGRAM_SCALE),
            fill,
            px(width * DIAGRAM_SCALE),
            stroke,
            Default::default(),
        ));
    }
}

fn alpha(color: Rgba, opacity: f32) -> Rgba {
    Rgba {
        a: color.a * opacity,
        ..color
    }
}

fn paint_network(bounds: Bounds<Pixels>, window: &mut Window, state: NetworkState, phase: f32) {
    let mut plot = Plot {
        origin: bounds.origin,
        window,
    };
    let clear = rgba(0x00000000);
    let active = state == NetworkState::Online;
    let accent = rgb(match state {
        NetworkState::Off | NetworkState::Direct => 0xb8aa9d,
        NetworkState::Preparing => ACCENT,
        NetworkState::Online => 0x259f87,
        NetworkState::Interrupted => 0xd09143,
    });
    plot.circle([160., 95.], 79., clear, alpha(accent, 0.07), 1.);
    plot.circle(
        [160., 95.],
        61.,
        alpha(accent, 0.025),
        alpha(accent, 0.09),
        1.,
    );
    for (index, route) in ROUTES.into_iter().enumerate() {
        let color = if active && index >= 2 {
            rgb(ACCENT)
        } else {
            accent
        };
        plot.curve(
            route,
            alpha(color, if active { 0.32 } else { 0.45 }),
            1.6,
            !active,
        );
        if active {
            let t = (phase * 2. + index as f32 * 0.25) % 1.;
            for trail in (0..7).rev() {
                let pos = curve_point(route, (t - trail as f32 * 0.018).rem_euclid(1.));
                plot.circle(
                    pos,
                    if trail == 0 { 2.6 } else { 1.7 },
                    alpha(color, 1. - trail as f32 / 8.),
                    clear,
                    0.,
                );
            }
            plot.circle(curve_point(route, t), 6., alpha(color, 0.07), clear, 0.);
        } else if state == NetworkState::Preparing {
            let t = (phase + index as f32 * 0.25) % 1.;
            plot.circle(curve_point(route, t), 2.4, alpha(accent, 0.7), clear, 0.);
        }
    }
    plot.circle([160., 95.], 33., rgb(0xffffff), alpha(accent, 0.23), 1.4);
    for center in [[34., 95.], [286., 95.]] {
        plot.circle(center, 24., rgb(0xffffff), alpha(accent, 0.24), 1.4);
        plot.circle(center, 20., alpha(accent, 0.045), clear, 0.);
    }
    let ink = if active { rgb(0x756456) } else { accent };
    plot.rect([22., 84.], [24., 16.], 2., clear, ink, 1.5);
    plot.line(&[[34., 100.], [34., 105.]], ink, 1.5);
    plot.line(&[[27., 105.], [41., 105.]], ink, 1.5);
    plot.circle([286., 95.], 12., clear, ink, 1.4);
    plot.line(&[[274., 95.], [298., 95.]], ink, 1.1);
    plot.curve(
        [[286., 83.], [277., 87.], [277., 103.], [286., 107.]],
        ink,
        1.1,
        false,
    );
    plot.curve(
        [[286., 83.], [295., 87.], [295., 103.], [286., 107.]],
        ink,
        1.1,
        false,
    );
    plot.circle([303., 112.], 9., rgb(0xffffff), clear, 0.);
    plot.circle([303., 112.], 7., accent, clear, 0.);
    let white = rgb(0xffffff);
    match state {
        NetworkState::Online => {
            plot.line(&[[299.5, 112.], [302., 114.5], [307., 109.5]], white, 1.5)
        }
        NetworkState::Off | NetworkState::Direct => {
            plot.line(&[[301., 109.], [301., 115.]], white, 1.4);
            plot.line(&[[305., 109.], [305., 115.]], white, 1.4);
        }
        NetworkState::Preparing => {
            plot.circle([301., 112.], 1., white, clear, 0.);
            plot.circle([305., 112.], 1., white, clear, 0.);
        }
        NetworkState::Interrupted => {
            plot.line(&[[303., 108.5], [303., 112.]], white, 1.5);
            plot.circle([303., 115.], 0.85, white, clear, 0.);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clashfx_core::model::{AppConfig, Candidate};

    #[::core::prelude::v1::test]
    fn ready_core_is_not_an_online_system_proxy_and_failures_stop_flow() {
        let mut snapshot = Snapshot::new(AppConfig::default(), String::new());
        snapshot.running = true;
        snapshot.active = Some(Candidate {
            key: "node".into(),
            name: "JP".into(),
            subscription_id: "sub".into(),
            subscription_name: "Test".into(),
        });
        assert_eq!(NetworkState::from_snapshot(&snapshot), NetworkState::Off);
        snapshot.system_proxy = true;
        assert_eq!(NetworkState::from_snapshot(&snapshot), NetworkState::Online);
        snapshot.config.routing_mode = RoutingMode::Direct;
        assert_eq!(NetworkState::from_snapshot(&snapshot), NetworkState::Direct);
        snapshot.config.routing_mode = RoutingMode::Rule;
        snapshot.phase = "池内节点全部不可用".into();
        assert_eq!(
            NetworkState::from_snapshot(&snapshot),
            NetworkState::Interrupted
        );
        snapshot.phase.clear();
        snapshot.probing = true;
        snapshot.failure_streak = 3;
        assert_eq!(
            NetworkState::from_snapshot(&snapshot),
            NetworkState::Preparing
        );
        snapshot.system_proxy = false;
        assert_eq!(NetworkState::from_snapshot(&snapshot), NetworkState::Off);
        snapshot.system_proxy = true;
        snapshot.running = false;
        assert_eq!(
            NetworkState::from_snapshot(&snapshot),
            NetworkState::Interrupted
        );
    }
}
