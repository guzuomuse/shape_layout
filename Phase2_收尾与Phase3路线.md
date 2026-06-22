# Phase 2 收尾 & Phase 3 路线 — 排版引擎 Y 推进策略改进

> 定案日期：2026-06-22
> 决策来源：两轮外部评审（共 15 条）+ 源代码深度审阅
> 原则：三问题，三分治，不混淆

---

## 问题全景

| # | 问题 | 严重度 | 根因 | 解决阶段 |
|:---:|:---|:---:|:---|:---:|
| 1 | **死区爬行浪费 CPU**：容器空区域逐像素探测 | 🟡 中 | `step_size=0.5` 在无区间区域无效微爬 | **Phase 2 P0 智能跳跃** |
| 2 | **kasuari 负数坐标**：元素是否被焊在 X ≥ 0 | ❓ 待验证 | 可能 kasuari 限制 / 可能 find_bottleneck 行为 | **Phase 2 Step 2 日志验证** |
| 3 | **心形上半空白**：元素只放在容器下半 | 🔴 高 | 单区间策略 + `max_by` 选最宽区间，无法利用多区域 | **Phase 3 P1 多区间并行** |

---

## Phase 2 收尾（今天，1 小时）

### Step 1: P0 智能跳跃（30 分钟）

**改动文件**：`engine.rs`

**改动位置**（3 处 `step_size` → `footprint_height().max(step_size)`）：

| 行号 | 场景 | 旧代码 | 新代码 |
|:---|:---|:---|:---|
| ~96 | `row_range.is_empty()` 无可用区间 | `y += config.step_size` | `y += elements[idx].footprint_height().max(config.step_size)` |
| ~116 | 区间太窄 (interval 宽度 ≤ 0) | `y += config.step_size` | `y += elements[idx].footprint_height().max(config.step_size)` |
| ~126 | 连第一个元素都放不下 | `y += row_height.max(config.step_size)` | `y += elements[idx].footprint_height().max(config.step_size)` |

**动机**：
- `get_intervals_at(y, row_height)` 扫描区间 `[y, y+row_height]`（非单点），P0 跳 `footprint_height()` 和微爬 `step_size=0.5` 的扫描区间只差 0.5 偏移
- 工业务形状在 0.5 单位内发生剧烈几何突变的概率极低 → "漏斗陷阱"被高估
- 效果：死区探测从 ~216 次降到 ~3 次

**不做**：
- 不跳 `footprint_height() / 4`（回复1 Round2 建议）— 无必要，区间扫描机制已消除风险
- 不跳 `2.0mm` 固定值 — 对高元素无效

### Step 2: 验证 kasuari 负数问题（30 分钟）

**改动**：在 `layout_rows` 添加日志，打印所有 `placed.x` 坐标

```rust
// 批量打印 placed X 坐标（验证 kasuari 是否限制非负）
println!("[kasusari_verify] placed X coords: {:?}",
    solution.placed.iter().map(|p| format!("{}={:.1}", p.id, p.x)).collect::<Vec<_>>()
);
let any_negative = solution.placed.iter().any(|p| p.x < -1e-9);
println!("[kasusari_verify] any X < 0: {}", any_negative);
```

**判断标准**：
- 如果有 `X < 0` → kasuari 没有问题，跳过修复
- 如果全部 `X ≥ 0` → 确认有问题，采用回复2的坐标平移方案

---

## Phase 3（下阶段核心，4-6 小时）

### P1 多区间并行

**根因**：心形上半有两个独立区域（左心房、右心房），单区间 `max_by` 只取最宽那个。如果元素宽度需求 > 单个心房宽度，整个上半被跳过。

**策略**：贪心选择最优区间（非简单每个区间独立排一行）

```
for y in container:
    intervals = get_intervals_at(y, row_height)
    对每个 interval 尝试 pack_row_elements
    选择放入元素最多的 interval
    其余 interval → 保存到候选池，稍后回溯
```

**不做的理由（Phase 3 再搞）**：
- 多区间语义复杂：哪个元素去左心房、哪个去右心房？需要分配策略
- 需要设计回溯机制（当前区间放不下时尝试其他区间）
- 回复5 指出：多区间并行在某些场景反而不如单区间逐行堆叠

---

## Phase 4+（推迟）

### P2 Jump Table

ROI 不高，两轮评审一致认为性价比不足。推迟到性能瓶颈实际出现时再做。

---

## 执行纪律

1. 每次改动后立即 `cargo check -p shape_layout`
2. 每步完成后写完工总结到 `完工总结/` 目录
3. P0 不混入 P1 语义——P0 只管"消除无效探测"，不管"让元素排到上半"
