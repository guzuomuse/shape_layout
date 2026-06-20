//! # 心形异形容器排版 Demo
//!
//! 演示 shape_layout Phase 2 全部功能：
//! - 异形容器（心形）
//! - SizeStrategy::Fixed / Fill
//! - VAlign::Baseline
//! - HAlign::Left / Center / Right
//! - Margin
//!
//! 运行：
//! ```bash
//! cargo run --example heart_demo -p shape_layout
//! ```

use shape_layout::{
    layout_rows, ElementMargin, HAlign, LayoutConfig, LayoutElement, SizeStrategy, VAlign,
};

use kurbo::BezPath;

/// 用参数方程构造心形 BezPath
///
/// 标准心形曲线：
///   x(t) = 16 sin³(t)
///   y(t) = 13 cos(t) - 5 cos(2t) - 2 cos(3t) - cos(4t)
///
/// t ∈ [0, 2π]，采样 N 个点用直线段连接。
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
        // 翻转 Y 轴使心形朝上（数学心形朝下）
        let py = cy - y * scale;

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

/// 构造一个带正方形孔洞的心形容器（模拟文章插图边界）
fn heart_with_hole(cx: f64, cy: f64, heart_scale: f64, hole_size: f64) -> BezPath {
    let mut path = heart_shape(cx, cy, heart_scale);
    // 在心形中心挖一个方形孔洞（CW 方向）
    let half = hole_size / 2.0;
    path.move_to((cx - half, cy + half));
    path.line_to((cx - half, cy - half));
    path.line_to((cx + half, cy - half));
    path.line_to((cx + half, cy + half));
    path.close_path();
    path
}

fn main() {
    println!("╔════════════════════════════════════════╗");
    println!("║  🫀 心形异形容器排版 Demo              ║");
    println!("║  SizeStrategy + Baseline + 心形       ║");
    println!("╚════════════════════════════════════════╝\n");

    // ── 场景 1: 基础心形容器 + 纯 Fixed 元素 ──
    {
        println!("━━━ 场景 1: 基础心形容器 + 混合对齐 ━━━");

        let heart = heart_with_hole(0.0, 0.0, 10.0, 60.0);
        // heart_scale=10, so heart is ~320 wide, ~300 tall, centered at (0,0)

        let mut logo = LayoutElement::new("logo", 80.0, 40.0);
        logo.margin = ElementMargin::uniform(5.0);

        let mut title = LayoutElement::new("title", 120.0, 30.0);
        title.margin = ElementMargin { top: 10.0, bottom: 5.0, left: 0.0, right: 0.0 };

        let desc = LayoutElement::new("description", 100.0, 20.0);

        let elements = vec![logo, title, desc];

        let config = LayoutConfig::with_spacing(8.0, 6.0, 6.0);

        let solution = layout_rows(&heart, &elements, &config);
        println!(
            "  排放: {}/{} 个元素",
            solution.placed_count(),
            solution.placed_count() + solution.unplaced.len()
        );
        for placed in &solution.placed {
            println!(
                "    {}: x={:.1}, y={:.1}, w={:.1}, h={:.1}",
                placed.id, placed.x, placed.y, placed.width, placed.height
            );
        }
        for id in &solution.unplaced {
            println!("    {}: ❌ 未排入", id);
        }
        for w in &solution.warnings {
            println!("    ⚠️  {:?}", w);
        }
        println!();
    }

    // ── 场景 2: SizeStrategy::Fill 弹性填充 ──
    {
        println!("━━━ 场景 2: Fill 弹性填充 ━━━");

        let heart = heart_shape(0.0, 0.0, 10.0);

        let fixed_logo = LayoutElement::new("logo", 60.0, 30.0);

        let mut fill_title = LayoutElement::new("title", 50.0, 25.0);
        fill_title.constraints.size_strategy = SizeStrategy::Fill;
        fill_title.constraints.max_width = Some(160.0); // 限制最大宽度

        let elements = vec![fixed_logo, fill_title];

        let config = LayoutConfig::with_spacing(10.0, 5.0, 10.0);

        let solution = layout_rows(&heart, &elements, &config);
        println!(
            "  排放: {}/{} 个元素",
            solution.placed_count(),
            solution.placed_count() + solution.unplaced.len()
        );
        for placed in &solution.placed {
            println!(
                "    {}: x={:.1}, y={:.1}, w={:.1}, h={:.1}",
                placed.id, placed.x, placed.y, placed.width, placed.height
            );
        }
        if !solution.unplaced.is_empty() {
            println!("  未排入: {:?}", solution.unplaced);
        }

        // 验证 Fill 确实拉伸了
        let fill_placed = solution.placed.iter().find(|p| p.id == "title").unwrap();
        assert!(
            fill_placed.width > 50.0,
            "Fill element should stretch, got width={}",
            fill_placed.width
        );
        println!("  ✅ Fill 元素自动拉伸到 w={:.1}\n", fill_placed.width);
    }

    // ── 场景 3: VAlign::Baseline 基线对齐 ──
    {
        println!("━━━ 场景 3: Baseline 基线对齐 ━━━");

        let heart = heart_shape(0.0, 0.0, 10.0);

        let mut text_large = LayoutElement::new("headline", 150.0, 60.0);
        text_large.baseline = Some(48.0); // 文字基线在接近底部
        text_large.margin = ElementMargin { top: 5.0, bottom: 5.0, left: 0.0, right: 0.0 };

        let mut icon = LayoutElement::new("icon", 40.0, 40.0);
        // icon 没有 baseline → 回退到 height（底部）对齐文本基线
        icon.margin = ElementMargin { top: 8.0, bottom: 4.0, left: 0.0, right: 0.0 };

        let elements = vec![text_large, icon];

        let mut config = LayoutConfig::with_spacing(10.0, 5.0, 10.0);
        config.valign = VAlign::Baseline;

        let solution = layout_rows(&heart, &elements, &config);
        println!(
            "  排放: {}/{} 个元素",
            solution.placed_count(),
            solution.placed_count() + solution.unplaced.len()
        );
        for placed in &solution.placed {
            println!(
                "    {}: x={:.1}, y={:.1}, w={:.1}, h={:.1}",
                placed.id, placed.x, placed.y, placed.width, placed.height
            );
        }

        // 验证基线对齐
        let headline_p = solution.placed.iter().find(|p| p.id == "headline").unwrap();
        let icon_p = solution.placed.iter().find(|p| p.id == "icon").unwrap();
        let headline_baseline_y = headline_p.y + 5.0 + 48.0; // y + margin.top + baseline
        let icon_baseline_y = icon_p.y + 8.0 + 40.0; // y + margin.top + effective_baseline(=height)
        assert!(
            (headline_baseline_y - icon_baseline_y).abs() < 1.0,
            "Baselines should align: text={}, icon={}",
            headline_baseline_y,
            icon_baseline_y
        );
        println!(
            "  ✅ 基线对齐：文字基线 Y={:.1}，图标底部 Y={:.1}\n",
            headline_baseline_y, icon_baseline_y
        );
    }

    // ── 场景 4: HAlign + Fill 混合 ──
    {
        println!("━━━ 场景 4: HAlign::Center + Fill ━━━");

        let heart = heart_shape(0.0, 0.0, 10.0);

        let mut fill_card = LayoutElement::new("card", 100.0, 50.0);
        fill_card.constraints.size_strategy = SizeStrategy::Fill;
        fill_card.constraints.max_width = Some(200.0); // 有上限的 Fill
        fill_card.constraints.min_width = Some(50.0);  // 允许缩小但不低于 50

        let elements = vec![fill_card];

        let mut config = LayoutConfig::with_spacing(10.0, 5.0, 10.0);
        config.halign = HAlign::Center;

        let solution = layout_rows(&heart, &elements, &config);
        assert!(solution.is_fully_placed());

        let placed = &solution.placed[0];

        // Fill + Center: 元素应在心形区间中居中（受 max_width 限制）
        // 验证 Fill 宽度不超过 max_width，且元素确实被排入
        assert!(placed.width <= 200.0 + 1.0,
            "Capped Fill width {} should not exceed max_width=200", placed.width);
        assert!(placed.width >= 50.0 - 1.0,
            "Capped Fill width {} should respect min_width=50", placed.width);
        println!(
            "  ✅ Capped Fill 居中: x={:.1}, w={:.1}\n",
            placed.x, placed.width
        );
    }

    // ── 场景 5: 综合场景 — 真实异形排版 ──
    {
        println!("━━━ 场景 5: 综合排版 ━━━");

        let heart = heart_shape(0.0, 0.0, 10.0);

        // 各种元素混合
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

        let solution = layout_rows(&heart, &elements, &config);
        println!(
            "  总元素: {}, 已排: {}, 未排: {}",
            solution.placed_count() + solution.unplaced.len(),
            solution.placed_count(),
            solution.unplaced.len()
        );

        for placed in &solution.placed {
            println!(
                "    ✅ {}: x={:.1}, y={:.1}, w={:.1}, h={:.1}",
                placed.id, placed.x, placed.y, placed.width, placed.height
            );
        }
        for id in &solution.unplaced {
            println!("    ❌ {}: 未排入", id);
        }
        for w in &solution.warnings {
            println!("    ⚠️  {:?}", w);
        }

        assert!(
            solution.placed_count() >= 3,
            "至少应排放 3 个元素，实际 {}",
            solution.placed_count()
        );
        println!("  ✅ 综合排版通过！\n");
    }

    println!("╔════════════════════════════════════════╗");
    println!("║  🎉 全部 5 个场景通过！               ║");
    println!("╚════════════════════════════════════════╝");
}
