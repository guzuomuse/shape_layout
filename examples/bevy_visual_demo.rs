//! # Shape Layout 可视化验证 Demo
//!
//! 用 Bevy 2D 渲染直观验证排版结果。
//!
//! ## 交互
//! - `1/2/3` 切换预设场景
//! - `Space` 随机生成元素并重排
//! - `Esc` 退出
//!
//! ## 颜色图例
//! - 蓝色 = Fixed (固定尺寸)
//! - 绿色 = Shrinkable (可收缩)
//! - 橙色 = Fill (弹性拉伸)
//!
//! ## 运行
//! ```bash
//! cargo run --example bevy_visual_demo -p shape_layout
//! ```

use bevy::prelude::*;
// Anchor removed: Bevy 0.18 Text2d uses internal TextLayout for alignment
use kurbo::BezPath;
use rand::RngExt;
use shape_layout::{
    layout_rows, ElementMargin, HAlign, LayoutConfig, LayoutElement, LayoutSolution, SizeStrategy,
    VAlign,
};


// ═══════════════════════════════════════════════════════════════
// 组件标记
// ═══════════════════════════════════════════════════════════════

/// 已排入元素的 Bevy Sprite 实体
#[derive(Component)]
struct PlacedElementRect;

/// UI 信息文本实体
#[derive(Component)]
struct InfoText;

/// 自定义字体句柄（避免中文乱码）
#[derive(Resource)]
struct DemoFont(Handle<Font>);

// ═══════════════════════════════════════════════════════════════
// 资源
// ═══════════════════════════════════════════════════════════════

/// 排版 Demo 全局状态
#[derive(Resource)]
struct LayoutDemoState {
    heart: BezPath,
    elements: Vec<LayoutElement>,
    config: LayoutConfig,
    solution: LayoutSolution,
    scene_label: String,
}

// ═══════════════════════════════════════════════════════════════
// 心形生成（与 heart_demo.rs 一致）
// ═══════════════════════════════════════════════════════════════

/// 心形缩放倍数（放大 2 倍，更清晰）
const HEART_SCALE: f64 = 20.0;

/// 用参数方程构造心形 BezPath
fn heart_shape(cx: f64, cy: f64, scale: f64) -> BezPath {
    const N: usize = 100;
    let mut path = BezPath::new();

    let mut first = true;
    for i in 0..N {
        let t = (i as f64) * std::f64::consts::TAU / (N as f64);
        let ct = t.cos();
        let st = t.sin();

        let x = 16.0 * st.powi(3);
        let y = 13.0 * ct - 5.0 * (2.0 * t).cos() - 2.0 * (3.0 * t).cos() - (4.0 * t).cos();

        let px = cx + x * scale;
        let py = cy - y * scale; // 翻转 Y 轴使心形朝上

        if first {
            path.move_to((px, py));
            first = false;
        } else {
            path.line_to((px, py));
        }
    }

    path.close_path();
    path
}

/// 从心形 BezPath 采样 N 个点用于 Gizmos 线框
fn sample_heart_points(_heart: &BezPath, n: usize) -> Vec<Vec2> {
    // 直接用参数方程重算（避免解析 BezPath 元素）
    let mut points = Vec::with_capacity(n + 1);
    for i in 0..n {
        let t = (i as f64) * std::f64::consts::TAU / (n as f64);
        let ct = t.cos();
        let st = t.sin();
        let x = 16.0 * st.powi(3);
        let y = 13.0 * ct - 5.0 * (2.0 * t).cos() - 2.0 * (3.0 * t).cos() - (4.0 * t).cos();
        // 翻转 Y（与 heart_shape 一致）
        points.push(Vec2::new((x * HEART_SCALE) as f32, (-y * HEART_SCALE) as f32));
    }
    // 闭合回第一个点
    points.push(points[0]);
    points
}

// ═══════════════════════════════════════════════════════════════
// 预设场景
// ═══════════════════════════════════════════════════════════════

/// 场景 1: 基础 Fixed 元素 + 混合对齐
fn scene_1() -> (Vec<LayoutElement>, LayoutConfig, &'static str) {
    let mut logo = LayoutElement::new("logo", 80.0, 40.0);
    logo.margin = ElementMargin::uniform(5.0);

    let mut title = LayoutElement::new("title", 120.0, 30.0);
    title.margin = ElementMargin {
        top: 10.0,
        bottom: 5.0,
        left: 0.0,
        right: 0.0,
    };

    let desc = LayoutElement::new("desc", 100.0, 20.0);

    let elements = vec![logo, title, desc];
    let config = LayoutConfig::with_spacing(8.0, 6.0, 6.0);
    (elements, config, "场景1: 纯Fixed + 混合对齐")
}

/// 场景 2: Fill 弹性拉伸
fn scene_2() -> (Vec<LayoutElement>, LayoutConfig, &'static str) {
    let fixed_logo = LayoutElement::new("logo", 60.0, 30.0);

    let mut fill_title = LayoutElement::new("title", 50.0, 25.0);
    fill_title.constraints.size_strategy = SizeStrategy::Fill;
    fill_title.constraints.max_width = Some(160.0);

    let elements = vec![fixed_logo, fill_title];
    let config = LayoutConfig::with_spacing(10.0, 5.0, 10.0);
    (elements, config, "场景2: Fixed + Fill")
}

/// 场景 3: 综合排版 (Baseline + Fill + Shrinkable)
fn scene_3() -> (Vec<LayoutElement>, LayoutConfig, &'static str) {
    let mut headline = LayoutElement::new("headline", 200.0, 40.0);
    headline.baseline = Some(32.0);

    let mut subtitle = LayoutElement::new("subtitle", 120.0, 25.0);
    subtitle.constraints.size_strategy = SizeStrategy::Fill;
    subtitle.baseline = Some(18.0);

    let mut body1 = LayoutElement::new("body1", 80.0, 20.0);
    body1.margin = ElementMargin::horizontal(10.0);

    let mut body2 = LayoutElement::new("body2", 60.0, 20.0);
    body2.constraints.size_strategy = SizeStrategy::Fixed { shrinkable: true };
    body2.constraints.min_width = Some(30.0);

    let mut body3 = LayoutElement::new("body3", 40.0, 20.0);
    body3.constraints.size_strategy = SizeStrategy::Fill;

    let elements = vec![headline, subtitle, body1, body2, body3];
    let mut config = LayoutConfig::with_spacing(10.0, 6.0, 8.0);
    config.valign = VAlign::Baseline;
    (elements, config, "场景3: 综合排版 (Baseline+Fill+Shrinkable)")
}

/// 随机场景
fn random_scene() -> (Vec<LayoutElement>, LayoutConfig, &'static str) {
    let mut rng = rand::rng();
    let count = rng.random_range(3..=8);

    let strategies = [
        SizeStrategy::Fixed { shrinkable: false },
        SizeStrategy::Fixed { shrinkable: true },
        SizeStrategy::Fill,
    ];

    let names = ["A", "B", "C", "D", "E", "F", "G", "H"];

    let mut elements = Vec::with_capacity(count);
    for i in 0..count {
        let w = rng.random_range(30.0..=150.0);
        let h = rng.random_range(15.0..=50.0);
        let strategy = strategies[rng.random_range(0..strategies.len())].clone();

        let mut elem = LayoutElement::new(names[i], w, h);
        elem.constraints.size_strategy = strategy;

        if rng.random_bool(0.3) {
            elem.margin = ElementMargin::uniform(rng.random_range(2.0..=8.0));
        }

        elements.push(elem);
    }

    // 随机对齐
    let valign = if rng.random_bool(0.5) {
        VAlign::Top
    } else {
        VAlign::Baseline
    };
    let halign = match rng.random_range(0..3) {
        0 => HAlign::Left,
        1 => HAlign::Center,
        _ => HAlign::Right,
    };

    let mut config = LayoutConfig::with_spacing(
        rng.random_range(4.0..=12.0),
        rng.random_range(3.0..=8.0),
        rng.random_range(3.0..=8.0),
    );
    config.valign = valign;
    config.halign = halign;

    (
        elements,
        config,
        "随机: 元素数/对齐/间距均为随机",
    )
}

// ═══════════════════════════════════════════════════════════════
// 辅助函数
// ═══════════════════════════════════════════════════════════════

/// 根据 SizeStrategy 返回对应颜色
fn element_color(strategy: &SizeStrategy) -> Color {
    match strategy {
        SizeStrategy::Fixed { shrinkable: false } => Color::srgba(0.2, 0.5, 1.0, 0.85), // 蓝
        SizeStrategy::Fixed { shrinkable: true } => Color::srgba(0.2, 0.8, 0.3, 0.85), // 绿
        SizeStrategy::Fill => Color::srgba(1.0, 0.45, 0.15, 0.85), // 橙
    }
}

/// 查找元素对应的策略
fn find_strategy<'a>(elements: &'a [LayoutElement], id: &str) -> &'a SizeStrategy {
    elements
        .iter()
        .find(|e| e.id == id)
        .map(|e| &e.constraints.size_strategy)
        .unwrap_or(&SizeStrategy::Fixed { shrinkable: false })
}

/// 清理旧的元素 Sprite，根据排版结果生成新的
fn respawn_element_sprites(
    commands: &mut Commands,
    solution: &LayoutSolution,
    elements: &[LayoutElement],
    font: &Handle<Font>,
) {
    for placed in &solution.placed {
        let strategy = find_strategy(elements, &placed.id);
        let color = element_color(strategy);

        // Sprite 的 Transform 中心点在矩形中心
        let center_x = (placed.x + placed.width / 2.0) as f32;
        let center_y = (placed.y + placed.height / 2.0) as f32;
        let w = placed.width as f32;
        let h = placed.height as f32;

        commands
            .spawn((
                Sprite {
                    color,
                    custom_size: Some(Vec2::new(w, h)),
                    ..default()
                },
                Transform::from_xyz(center_x, center_y, 1.0),
                PlacedElementRect,
            ))
            .with_children(|parent| {
                // 文字标签：居中锚点，内偏移定位在左上角附近
                let label_font_size = (h.min(16.0) - 2.0).max(8.0);
                let text_x = -w / 2.0 + 5.0;
                let text_y = h / 2.0 - label_font_size / 2.0 - 2.0;
                parent.spawn((
                    Text2d::new(placed.id.clone()),
                    TextFont {
                        font: font.clone(),
                        font_size: label_font_size,
                        ..default()
                    },
                    TextColor(Color::WHITE),
                    Transform::from_xyz(text_x, text_y, 0.1),
                ));
            });
    }
}

// ═══════════════════════════════════════════════════════════════
// 系统
// ═══════════════════════════════════════════════════════════════

/// 启动系统：相机 + UI + 初始排版
fn setup(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
) {
    // 2D 相机
    commands.spawn(Camera2d);

    // 加载中文字体
    let font_handle: Handle<Font> = asset_server.load("font/default.ttf");
    let font = font_handle.clone();

    // 标题文字
    commands.spawn((
        Text2d::new("Shape Layout Visual Validator"),
        TextFont {
            font: font.clone(),
            font_size: 20.0,
            ..default()
        },
        TextColor(Color::WHITE),
        Transform::from_xyz(-380.0, 370.0, 10.0),
        InfoText,
    ));

    // 操作提示
    commands.spawn((
        Text2d::new("[1/2/3] Scenes  [Space] Random  [Esc] Quit"),
        TextFont {
            font: font.clone(),
            font_size: 14.0,
            ..default()
        },
        TextColor(Color::srgba(0.7, 0.7, 0.7, 1.0)),
        Transform::from_xyz(-380.0, 340.0, 10.0),
        InfoText,
    ));

    // 颜色图例
    commands.spawn((
        Text2d::new("Legend: Blue=Fixed  Green=Shrinkable  Orange=Fill"),
        TextFont {
            font: font.clone(),
            font_size: 13.0,
            ..default()
        },
        TextColor(Color::srgba(0.8, 0.8, 0.8, 1.0)),
        Transform::from_xyz(-380.0, 315.0, 10.0),
        InfoText,
    ));

    // 场景名称文本（动态更新）
    commands.spawn((
        Text2d::new(""),
        TextFont {
            font: font.clone(),
            font_size: 15.0,
            ..default()
        },
        TextColor(Color::srgba(0.5, 0.9, 0.5, 1.0)),
        Transform::from_xyz(-380.0, 285.0, 10.0),
        InfoText,
    ));

    // 初始布局：场景 1
    let heart = heart_shape(0.0, 0.0, HEART_SCALE);
    let (elements, config, label) = scene_1();
    let solution = layout_rows(&heart, &elements, &config);
    respawn_element_sprites(&mut commands, &solution, &elements, &font);

    commands.insert_resource(DemoFont(font));
    commands.insert_resource(LayoutDemoState {
        heart,
        elements,
        config,
        solution,
        scene_label: label.to_string(),
    });
}

/// 键盘处理：切换场景 + 更新排版
fn handle_keyboard(
    mut commands: Commands,
    keyboard: Res<ButtonInput<KeyCode>>,
    mut state: ResMut<LayoutDemoState>,
    element_query: Query<Entity, With<PlacedElementRect>>,
    mut text_query: Query<&mut Text2d, With<InfoText>>,
    font: Res<DemoFont>,
) {
    let mut new_scene: Option<usize> = None;

    if keyboard.just_pressed(KeyCode::Digit1) {
        new_scene = Some(1);
    } else if keyboard.just_pressed(KeyCode::Digit2) {
        new_scene = Some(2);
    } else if keyboard.just_pressed(KeyCode::Digit3) {
        new_scene = Some(3);
    } else if keyboard.just_pressed(KeyCode::Space) {
        new_scene = Some(0); // 0 = random
    } else if keyboard.just_pressed(KeyCode::Escape) {
        // 退出应用
        std::process::exit(0);
    }

    let Some(scene_id) = new_scene else {
        return;
    };

    // 1. 清理旧 Sprite
    for entity in &element_query {
        commands.entity(entity).despawn();
    }

    // 2. 生成新排版
    let (elements, config, label) = match scene_id {
        1 => scene_1(),
        2 => scene_2(),
        3 => scene_3(),
        _ => random_scene(),
    };

    let solution = layout_rows(&state.heart, &elements, &config);

    // 3. 生成新 Sprite
    respawn_element_sprites(&mut commands, &solution, &elements, &font.0);

    // 4. 更新场景名文本
    for mut text in &mut text_query {
        let s = text.0.as_str();
        if s.is_empty() || s.starts_with("Scene") || s.starts_with("Random") {
            text.0 = format!(
                "Scene: {} | Placed {}/{}",
                label,
                solution.placed_count(),
                solution.placed_count() + solution.unplaced.len()
            );
        }
    }

    // 5. 更新状态
    state.elements = elements;
    state.config = config;
    state.solution = solution;
    state.scene_label = label.to_string();
}

/// 每帧绘制心形线框（使用 Gizmos）
fn draw_heart_gizmo(mut gizmos: Gizmos, state: Res<LayoutDemoState>) {
    let points = sample_heart_points(&state.heart, 120);
    gizmos.linestrip_2d(points, Color::srgba(0.9, 0.9, 0.9, 0.7));
}

// ═══════════════════════════════════════════════════════════════
// 入口
// ═══════════════════════════════════════════════════════════════

fn main() {
    App::new()
        .add_plugins(DefaultPlugins)
        .add_systems(Startup, setup)
        .add_systems(Update, (handle_keyboard, draw_heart_gizmo))
        .run();
}
