//! # Shape Layout 可视化验证 Demo
//!
//! 用 Bevy 2D 渲染直观验证排版结果。
//!
//! ## 交互
//! - `1~6` 切换预设形状场景（Heart / Rect / Circle / Gourd / HangTag / RoundedRect）
//! - `Space` 随机生成元素并重排
//! - `R` 从 `viz_state.ron` 强制重载（MCP 桥接文件）
//! - `Esc` 退出
//!
//! ## 颜色图例
//! - 蓝色 = Fixed (固定尺寸)
//! - 绿色 = Shrinkable (可收缩)
//! - 橙色 = Fill (弹性拉伸)
//!
//! ## 运行
//! ```bash
//! # 手动场景模式
//! cargo run --example bevy_visual_demo -p shape_layout
//!
//! # MCP 桥接模式（配合 shape_layout_mcp 使用）
//! set SHAPE_LAYOUT_VIZ_FILE=./viz_state.ron
//! cargo run --example bevy_visual_demo -p shape_layout
//! ```

use bevy::ecs::schedule::common_conditions::run_once;
use bevy::prelude::*;
use rand::RngExt;
use shape_layout::{
    layout_container, ContainerShape, ElementMargin, HAlign, LayoutConfig, LayoutElement,
    LayoutSolution, LayoutVizState, SizeStrategy, VAlign,
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

/// 容器轮廓 Mesh2d 实体
#[derive(Component)]
struct ContainerOutline;

/// 自定义字体句柄（避免中文乱码）
#[derive(Resource)]
struct DemoFont(Handle<Font>);

// ═══════════════════════════════════════════════════════════════
// 资源
// ═══════════════════════════════════════════════════════════════

/// 排版 Demo 全局状态
#[derive(Resource)]
struct LayoutDemoState {
    container: ContainerShape,
    elements: Vec<LayoutElement>,
    config: LayoutConfig,
    solution: LayoutSolution,
    scene_label: String,
}

/// MCP 可视化桥文件路径
#[derive(Resource)]
struct VizFilePath(String);

// ═══════════════════════════════════════════════════════════════
// 预设场景
// ═══════════════════════════════════════════════════════════════

/// 场景 1: 大量元素填满心形
fn scene_heart() -> (ContainerShape, Vec<LayoutElement>, LayoutConfig, String) {
    let mut elements = Vec::new();
    let names = [
        "A", "B", "C", "D", "E", "F", "G", "H", "I", "J", "K", "L", "M", "N", "O", "P", "Q",
        "R", "S", "T", "U", "V", "W", "X", "Y", "Z", "a1", "b2", "c3", "d4", "e5", "f6", "g7",
        "h8", "i9",
    ];
    let sizes = [
        (80.0, 35.0),
        (120.0, 28.0),
        (65.0, 22.0),
        (95.0, 30.0),
        (50.0, 18.0),
        (110.0, 25.0),
        (70.0, 20.0),
        (40.0, 16.0),
        (130.0, 32.0),
        (55.0, 24.0),
        (90.0, 20.0),
        (75.0, 28.0),
        (100.0, 22.0),
        (45.0, 18.0),
        (85.0, 30.0),
        (60.0, 20.0),
        (140.0, 35.0),
        (35.0, 15.0),
        (105.0, 26.0),
        (70.0, 22.0),
        (50.0, 20.0),
        (90.0, 25.0),
        (65.0, 18.0),
        (80.0, 22.0),
        (120.0, 28.0),
        (45.0, 16.0),
        (95.0, 24.0),
        (55.0, 20.0),
        (75.0, 25.0),
        (40.0, 18.0),
        (100.0, 30.0),
        (60.0, 22.0),
        (85.0, 20.0),
        (110.0, 28.0),
        (70.0, 24.0),
    ];
    for (i, ((w, h), name)) in sizes.iter().zip(names.iter()).enumerate() {
        let mut elem = LayoutElement::new(*name, *w, *h);
        if i % 4 == 0 {
            elem.margin = ElementMargin::uniform(4.0);
        }
        if i % 5 == 0 {
            elem.constraints.size_strategy = SizeStrategy::Fill;
            elem.constraints.max_width = Some(160.0);
        }
        if i % 7 == 0 {
            elem.constraints.size_strategy = SizeStrategy::Fixed { shrinkable: true };
            elem.constraints.min_width = Some(20.0);
        }
        elements.push(elem);
    }
    let config = LayoutConfig::with_spacing(6.0, 4.0, 4.0);
    (
        ContainerShape::Heart { width: 220.0 },
        elements,
        config,
        "场景1: 心形 35元素".to_string(),
    )
}

/// 场景 2: 名片 Rect + Fixed + Fill
fn scene_rect() -> (ContainerShape, Vec<LayoutElement>, LayoutConfig, String) {
    let fixed_logo = LayoutElement::new("logo", 60.0, 30.0);
    let mut fill_title = LayoutElement::new("title", 50.0, 25.0);
    fill_title.constraints.size_strategy = SizeStrategy::Fill;
    fill_title.constraints.max_width = Some(160.0);
    let elements = vec![fixed_logo, fill_title];
    let config = LayoutConfig::with_spacing(10.0, 5.0, 10.0);
    (
        ContainerShape::Rect {
            width: 200.0,
            height: 100.0,
        },
        elements,
        config,
        "场景2: 矩形 Fixed + Fill".to_string(),
    )
}

/// 场景 3: 圆形徽章
fn scene_circle() -> (ContainerShape, Vec<LayoutElement>, LayoutConfig, String) {
    let headline = LayoutElement::new("headline", 200.0, 40.0);
    let mut subtitle = LayoutElement::new("subtitle", 120.0, 25.0);
    subtitle.constraints.size_strategy = SizeStrategy::Fill;
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
    (
        ContainerShape::Circle { diameter: 180.0 },
        elements,
        config,
        "场景3: 圆形 综合排版".to_string(),
    )
}

/// 场景 4: 葫芦瓶标
fn scene_gourd() -> (ContainerShape, Vec<LayoutElement>, LayoutConfig, String) {
    let elements = vec![
        LayoutElement::new("brand", 40.0, 20.0),
        LayoutElement::new("year", 35.0, 18.0),
        LayoutElement::new("region", 50.0, 18.0),
        LayoutElement::new("alc", 30.0, 16.0),
        LayoutElement::new("vol", 60.0, 20.0),
    ];
    let config = LayoutConfig::with_spacing(6.0, 3.0, 3.0);
    (
        ContainerShape::Gourd {
            width: 120.0,
            height: 180.0,
            waist_y: 0.55,
            waist_ratio: 0.45,
        },
        elements,
        config,
        "场景4: 葫芦 5元素".to_string(),
    )
}

/// 场景 5: 吊牌标签
fn scene_hangtag() -> (ContainerShape, Vec<LayoutElement>, LayoutConfig, String) {
    let elements = vec![
        LayoutElement::new("brand", 50.0, 25.0),
        LayoutElement::new("size", 40.0, 20.0),
    ];
    let config = LayoutConfig {
        padding_top: 10.0,
        padding_bottom: 10.0,
        padding_left: 8.0,
        padding_right: 8.0,
        gap: 5.0,
        line_spacing: 5.0,
        halign: HAlign::Center,
        ..LayoutConfig::default()
    };
    (
        ContainerShape::HangTag {
            width: 80.0,
            height: 120.0,
            radius: 5.0,
            hole_y: 100.0,
            hole_radius: 6.0,
        },
        elements,
        config,
        "场景5: 吊牌 2元素".to_string(),
    )
}

/// 场景 6: 圆角矩形
fn scene_rounded_rect() -> (ContainerShape, Vec<LayoutElement>, LayoutConfig, String) {
    let mut elements = Vec::new();
    let names = ["logo", "title", "subtitle", "badge", "tag"];
    let sizes = [
        (60.0, 35.0),
        (140.0, 30.0),
        (100.0, 22.0),
        (50.0, 20.0),
        (70.0, 20.0),
    ];
    for (name, (w, h)) in names.iter().zip(sizes.iter()) {
        let mut elem = LayoutElement::new(*name, *w, *h);
        if *name == "tag" {
            elem.constraints.size_strategy = SizeStrategy::Fill;
        }
        elements.push(elem);
    }
    let config = LayoutConfig::with_spacing(12.0, 6.0, 6.0);
    (
        ContainerShape::RoundedRect {
            width: 200.0,
            height: 150.0,
            radius: 20.0,
        },
        elements,
        config,
        "场景6: 圆角矩形 5元素".to_string(),
    )
}

/// 随机场景
fn random_scene() -> (ContainerShape, Vec<LayoutElement>, LayoutConfig, String) {
    let mut rng = rand::rng();
    let count = rng.random_range(15..=30);
    let strategies = [
        SizeStrategy::Fixed {
            shrinkable: false,
        },
        SizeStrategy::Fixed { shrinkable: true },
        SizeStrategy::Fill,
    ];
    let names = [
        "A", "B", "C", "D", "E", "F", "G", "H", "I", "J", "K", "L", "M", "N", "O", "P", "Q",
        "R", "S", "T", "U", "V", "W", "X", "Y", "Z", "a1", "b2", "c3", "d4",
    ];

    let mut elements = Vec::with_capacity(count);
    for i in 0..count {
        let w = rng.random_range(30.0..=150.0);
        let h = rng.random_range(15.0..=45.0);
        let strategy = strategies[rng.random_range(0..strategies.len())].clone();
        let mut elem = LayoutElement::new(names[i % names.len()], w, h);
        elem.constraints.size_strategy = strategy;
        if rng.random_bool(0.3) {
            elem.margin = ElementMargin::uniform(rng.random_range(2.0..=8.0));
        }
        elements.push(elem);
    }

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

    // 随机容器形状
    let container = match rng.random_range(0..6) {
        0 => ContainerShape::Heart { width: 200.0 },
        1 => ContainerShape::Rect {
            width: 200.0,
            height: 140.0,
        },
        2 => ContainerShape::Circle { diameter: 180.0 },
        3 => ContainerShape::Gourd {
            width: 120.0,
            height: 180.0,
            waist_y: 0.55,
            waist_ratio: 0.45,
        },
        4 => ContainerShape::HangTag {
            width: 80.0,
            height: 120.0,
            radius: 5.0,
            hole_y: 100.0,
            hole_radius: 6.0,
        },
        _ => ContainerShape::RoundedRect {
            width: 200.0,
            height: 150.0,
            radius: 20.0,
        },
    };

    (
        container,
        elements,
        config,
        "随机: 元素/形状/对齐均随机".to_string(),
    )
}

// ═══════════════════════════════════════════════════════════════
// 辅助函数
// ═══════════════════════════════════════════════════════════════

/// 根据 SizeStrategy 返回对应颜色
fn element_color(strategy: &SizeStrategy) -> Color {
    match strategy {
        SizeStrategy::Fixed {
            shrinkable: false,
        } => Color::srgba(0.2, 0.5, 1.0, 0.85),
        SizeStrategy::Fixed { shrinkable: true } => Color::srgba(0.2, 0.8, 0.3, 0.85),
        SizeStrategy::Fill => Color::srgba(1.0, 0.45, 0.15, 0.85),
    }
}

/// 查找元素对应的策略
fn find_strategy<'a>(elements: &'a [LayoutElement], id: &str) -> &'a SizeStrategy {
    elements
        .iter()
        .find(|e| e.id == id)
        .map(|e| &e.constraints.size_strategy)
        .unwrap_or(&SizeStrategy::Fixed {
            shrinkable: false,
        })
}

/// 生成已排元素的 Sprite 实体
fn respawn_element_sprites(
    commands: &mut Commands,
    solution: &LayoutSolution,
    elements: &[LayoutElement],
    font: &Handle<Font>,
) {
    for placed in &solution.placed {
        let strategy = find_strategy(elements, &placed.id);
        let color = element_color(strategy);

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

/// 生成容器轮廓 Mesh2d 实体（使用 pce_lyon 平滑渲染）
fn spawn_container_outline(
    commands: &mut Commands,
    meshes: &mut ResMut<Assets<Mesh>>,
    materials: &mut ResMut<Assets<ColorMaterial>>,
    container: &ContainerShape,
) {
    let bezpath = container.to_bezpath();
    let mesh = pce_lyon::build_stroke_mesh(
        &bezpath,
        2.0,                  // 线宽
        [0.9, 0.9, 0.9, 0.7], // 灰白色半透明
        0.1,                   // 精度
    );

    if mesh.count_vertices() == 0 {
        return; // 退化路径
    }

    let mesh_handle = meshes.add(mesh);
    let material_handle = materials.add(ColorMaterial::default());

    commands.spawn((
        Mesh2d(mesh_handle),
        MeshMaterial2d(material_handle),
        Transform::from_xyz(0.0, 0.0, 0.0),
        ContainerOutline,
    ));
}

/// 更新场景名称文本
fn update_info_text(
    text_query: &mut Query<&mut Text2d, With<InfoText>>,
    label: &str,
    placed: usize,
    total: usize,
) {
    for mut text in text_query {
        let s = text.0.as_str();
        // 只更新场景名行和状态行（不碰标题/图例固定文本）
        if s.is_empty()
            || s.starts_with("Scene")
            || s.starts_with("Random")
            || s.starts_with("MCP")
            || s.starts_with("[Scene")
            || s == "Loading..."
        {
            text.0 = format!("[Scene] {label} | Placed {placed}/{total}");
        }
    }
}

/// 清理旧实体并重建整个场景（共享函数，键盘/文件监听均调用）
fn rebuild_scene(
    commands: &mut Commands,
    meshes: &mut ResMut<Assets<Mesh>>,
    materials: &mut ResMut<Assets<ColorMaterial>>,
    state: &mut ResMut<LayoutDemoState>,
    container: ContainerShape,
    elements: Vec<LayoutElement>,
    config: LayoutConfig,
    label: String,
    element_query: &Query<Entity, With<PlacedElementRect>>,
    outline_query: &Query<Entity, With<ContainerOutline>>,
    text_query: &mut Query<&mut Text2d, With<InfoText>>,
    font: &Res<DemoFont>,
) {
    // 1. 清理旧实体
    for entity in element_query {
        commands.entity(entity).despawn();
    }
    for entity in outline_query {
        commands.entity(entity).despawn();
    }

    // 2. 执行排版
    let solution = layout_container(&container, &elements, &config);
    let total = solution.placed_count() + solution.unplaced.len();

    // 3. 生成新 Sprite
    respawn_element_sprites(commands, &solution, &elements, &font.0);

    // 4. 生成容器轮廓 Mesh2d
    spawn_container_outline(commands, meshes, materials, &container);

    // 5. 更新 UI 文本
    update_info_text(text_query, &label, solution.placed_count(), total);

    // 6. 更新状态
    state.container = container;
    state.elements = elements;
    state.config = config;
    state.solution = solution;
    state.scene_label = label;
}

// ═══════════════════════════════════════════════════════════════
// 系统
// ═══════════════════════════════════════════════════════════════

/// 启动系统：相机 + UI + 初始排版
fn setup(mut commands: Commands, asset_server: Res<AssetServer>) {
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
        Text2d::new("[1-6] Scenes  [Space] Random  [R] Reload  [Esc] Quit"),
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
        Text2d::new("Loading..."),
        TextFont {
            font: font.clone(),
            font_size: 15.0,
            ..default()
        },
        TextColor(Color::srgba(0.5, 0.9, 0.5, 1.0)),
        Transform::from_xyz(-380.0, 285.0, 10.0),
        InfoText,
    ));

    // 初始化场景：优先从 viz_state.ron 加载，否则用场景 1
    let viz_path =
        std::env::var("SHAPE_LAYOUT_VIZ_FILE").unwrap_or_else(|_| "./viz_state.ron".to_string());
    let (container, elements, config, label) =
        if let Ok(ron_str) = std::fs::read_to_string(&viz_path) {
            if let Ok(viz_state) = ron::from_str::<LayoutVizState>(&ron_str) {
                let elements = viz_state.elements;
                let label = format!("MCP: {viz_path}");
                (
                    viz_state.container,
                    elements,
                    viz_state.config,
                    label,
                )
            } else {
                scene_heart()
            }
        } else {
            scene_heart()
        };

    let solution = layout_container(&container, &elements, &config);

    // 初始元素 Sprite（在 startup 阶段直接 spawn，等 rebuild_scene 无法用 Query）
    respawn_element_sprites(&mut commands, &solution, &elements, &font);

    commands.insert_resource(DemoFont(font));
    commands.insert_resource(LayoutDemoState {
        container,
        elements,
        config,
        solution,
        scene_label: label.clone(),
    });
    commands.insert_resource(VizFilePath(viz_path));

    // 启动时容器轮廓留到第一个 Update 帧渲染（需要 Meshes/Materials 资源就绪）
}

/// 键盘处理：切换场景 / 随机 / 重载文件
#[allow(clippy::too_many_arguments)]
fn handle_keyboard(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
    keyboard: Res<ButtonInput<KeyCode>>,
    viz_path: Res<VizFilePath>,
    mut state: ResMut<LayoutDemoState>,
    element_query: Query<Entity, With<PlacedElementRect>>,
    outline_query: Query<Entity, With<ContainerOutline>>,
    mut text_query: Query<&mut Text2d, With<InfoText>>,
    font: Res<DemoFont>,
) {
    let scene: Option<(ContainerShape, Vec<LayoutElement>, LayoutConfig, String)> =
        if keyboard.just_pressed(KeyCode::Digit1) {
            Some(scene_heart())
        } else if keyboard.just_pressed(KeyCode::Digit2) {
            Some(scene_rect())
        } else if keyboard.just_pressed(KeyCode::Digit3) {
            Some(scene_circle())
        } else if keyboard.just_pressed(KeyCode::Digit4) {
            Some(scene_gourd())
        } else if keyboard.just_pressed(KeyCode::Digit5) {
            Some(scene_hangtag())
        } else if keyboard.just_pressed(KeyCode::Digit6) {
            Some(scene_rounded_rect())
        } else if keyboard.just_pressed(KeyCode::Space) {
            Some(random_scene())
        } else if keyboard.just_pressed(KeyCode::KeyR) {
            // 从 viz 文件强制重载
            if let Ok(ron_str) = std::fs::read_to_string(&viz_path.0) {
                if let Ok(viz_state) = ron::from_str::<LayoutVizState>(&ron_str) {
                    Some((
                        viz_state.container,
                        viz_state.elements,
                        viz_state.config,
                        format!("MCP: {}", viz_path.0),
                    ))
                } else {
                    None
                }
            } else {
                None
            }
        } else if keyboard.just_pressed(KeyCode::Escape) {
            std::process::exit(0);
        } else {
            None
        };

    if let Some((container, elements, config, label)) = scene {
        rebuild_scene(
            &mut commands,
            &mut meshes,
            &mut materials,
            &mut state,
            container,
            elements,
            config,
            label,
            &element_query,
            &outline_query,
            &mut text_query,
            &font,
        );
    }
}

/// first-frame 系统：在首次 Update 时生成容器轮廓
fn first_frame_outline(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
    state: Res<LayoutDemoState>,
) {
    spawn_container_outline(&mut commands, &mut meshes, &mut materials, &state.container);
}

/// 文件监听：viz_state.ron 变化时自动重载
#[allow(clippy::too_many_arguments)]
fn watch_viz_file(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
    viz_path: Res<VizFilePath>,
    mut last_mtime: Local<Option<std::time::SystemTime>>,
    mut state: ResMut<LayoutDemoState>,
    element_query: Query<Entity, With<PlacedElementRect>>,
    outline_query: Query<Entity, With<ContainerOutline>>,
    mut text_query: Query<&mut Text2d, With<InfoText>>,
    font: Res<DemoFont>,
) {
    let meta = match std::fs::metadata(&viz_path.0) {
        Ok(m) => m,
        Err(_) => return,
    };
    let mtime = match meta.modified() {
        Ok(t) => t,
        Err(_) => return,
    };
    if *last_mtime == Some(mtime) {
        return;
    }
    *last_mtime = Some(mtime);

    let ron_str = match std::fs::read_to_string(&viz_path.0) {
        Ok(s) => s,
        Err(_) => return,
    };
    let viz_state = match ron::from_str::<LayoutVizState>(&ron_str) {
        Ok(s) => s,
        Err(_) => return,
    };

    rebuild_scene(
        &mut commands,
        &mut meshes,
        &mut materials,
        &mut state,
        viz_state.container,
        viz_state.elements,
        viz_state.config,
        format!("MCP: {}", viz_path.0),
        &element_query,
        &outline_query,
        &mut text_query,
        &font,
    );
}

// ═══════════════════════════════════════════════════════════════
// 入口
// ═══════════════════════════════════════════════════════════════

fn main() {
    App::new()
        .add_plugins(DefaultPlugins)
        .add_systems(Startup, setup)
        .add_systems(
            Update,
            (
                first_frame_outline.run_if(run_once),
                handle_keyboard,
                watch_viz_file,
            ),
        )
        .run();
}
