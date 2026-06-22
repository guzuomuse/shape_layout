# shape_layout Y 推进策略：现状分析与改进方案

> 版本: 2026-06-22  
> 状态: 供评审讨论  

---

## 目录

1. [当前 Y 推进逻辑全景](#一当前-y-推进逻辑全景)
2. [设计决策与理由](#二设计决策与理由)
3. [已知问题与根因分析](#三已知问题与根因分析)
4. [改进方案](#四改进方案)
5. [可选方案对比](#五可选方案对比)
6. [建议路线](#六建议路线)

---

## 一、当前 Y 推进逻辑全景

### 1.1 入口与初始化

```
y = container.y0 + padding_top    // 从容器顶部 + 上边距开始
max_y = container.y1 - padding_bottom  // 容器底部 - 下边距（硬天花板）
idx = 0                            // 当前待排元素指针
```

### 1.2 主循环（每行一次迭代）

```
while idx < elements.len():

  【0】底部溢出检查
       if y > max_y → 剩余全部元素 → unplaced + Warning → break

  【1】初始行高估算
       row_height = elements[idx].height   // 用当前元素高度做初始估计

  【2】查询行区间
       row_range = rg.get_intervals_at(y, row_height, config.min_width)
       
       if row_range.is_empty():  →  y += config.step_size  →  continue  
          ↑ 这是"死区爬行"——Y 按微步长 0.5 一点一点往上挪

  【3】取最宽区间
       widest = max_by_width(row_range.intervals)
       interval_l = widest.0 + padding_left
       interval_r = widest.1 - padding_right
       
       if interval_width <= 0:  →  y += config.step_size  →  continue

  【4】贪心打包行内元素
       (row_indices, new_idx, final_row_height) = pack_row_elements(…)
       
       if row_indices.is_empty():  →  y += row_height.max(config.step_size)  →  continue
          ↑ 单元素太宽，放不下

  【5】基线行高修正（仅 VAlign::Baseline）
       根据 max_ascent + max_descent 重新计算 row_height

  【6】Refinement：用实际行高重查区间
       若 refinement 后区间太窄（沙漏形容器问题）→ 回退到原始区间

  【7】底部溢出二次确认
       if y + row_height > max_y → 整行 unplaced + Warning → break

  【8】kasuari 求解行内 X
       Ok(x_solutions) → 记录 placed 元素
       Err → 整行 unplaced + Warning

  【9】Y 推进
       y += row_height + config.line_spacing
```

### 1.3 行打包逻辑 `pack_row_elements`

```
以 start_idx 开始，贪心尝试放入元素：
  for elem in elements[start_idx..]:
    首选宽度 = elem.footprint_width()
    
    if 首选宽度 + gap + 已用宽度 <= 可用宽度:
      放入行内
    
    elif elem.can_shrink() and min_宽度 + gap + 已用宽度 <= 可用宽度:
      以最小宽度放入行内
    
    else:
      break  // 此行到此为止，剩余元素走下一行
```

### 1.4 区间查询逻辑 `get_intervals_at`

```
对给定的 (y_start, height):
  1. 构造切片矩形（略宽于容器 AABB）
  2. 外轮廓 AND 切片矩形 → 基础安全区间
  3. 孔洞裁剪（subtract 孔洞区间）
  4. min_width 过滤 + X 升序排列
  5. 返回 RowRange { y_start, height, intervals: [(l,r), …] }
  
  心形等高线示例：
    Y≈-238 (尖端):  intervals=[]         → 宽度≈0
    Y≈-200:         intervals=[(l,r)]   → 单个区间（心形尚未分叉）
    Y≈-150 (分叉处): intervals=[(l1,r1), (l2,r2)]  → 左右两个心房
    Y≈-50 (顶部):    intervals=[(l,r)]   → 重新合并为一个区间
```

---

## 二、设计决策与理由

### 2.1 为什么用贪心分行而不是全局 DP？

- **理由 1**：输入元素有顺序语义（设计师有意识地把"标题→副标题→正文"排好序）
- **理由 2**：贪心是线性时间 O(n)，DP 是 O(n²) 或更高——WASM 环境下不可接受
- **理由 3**：Figma/CSS 也是贪心流式而非全局优化

### 2.2 为什么"取最宽区间"而不是"多区间并行"？

- **理由**：多区间并行本质上是把一行元素分配进多个不连通的区间，这增加了：
  - 求解器复杂度（跨区间 gap 不可用普通 gap 处理）
  - 元素分配策略（哪个元素去左心房？哪个去右心房？）
- **代价**：浪费了另一半可用空间，元素挤在一侧
- **历史**：这是 Phase 2 简化决策，多区间被推迟到 Phase 3+

### 2.3 为什么 `step_size` 默认 0.5？

- **理由**：精确定位到容器可用区域，不错过任何一个可能放得下行元素的 Y 位置
- **代价**：在尖角/死区（区间宽度 ≈ 0~10）爬行极慢，白白消耗垂直空间

### 2.4 为什么有两处不同的 Y 推进量？

| 位置 | 推进量 | 语义 |
|:---|:---|:---|
| `row_range.is_empty()` 后 | `config.step_size` (0.5) | 死区：没找到任何可用区间，微步探测 |
| `row_indices.is_empty()` 后 | `row_height.max(config.step_size)` | 元素太宽放不下：至少跳过这个元素的高度 |
| 正常行排放后 | `row_height + config.line_spacing` | 行高 + 行间距 |

**不一致**：死区情况用了固定的 0.5，但在死区中 `elements[idx].height` 可能 > 0.5——意味着白白做了几百次无效探测。

### 2.5 行高估算为什么从 `elements[idx].height` 开始？

- **理由**：至少要有当前元素的高度才能放得下它
- **问题**：心形底部尖角区域，宽度可能不足以放下哪怕 1 个像素高的元素——用 0.5 做行高去探测已经足够了
- **但**：一旦离开尖角区域（比如 Y=-200 处已有 20 个单位宽度），就应该用实际行高去探测，而非 0.5

---

## 三、已知问题与根因分析

### 3.1 问题 A：死区 Y 爬行消耗上半空间

**现象**：心形上半部分完全空白，元素全部挤在下半部分

**根因链**：

```
心形底部 Y≈-238（尖端）：
  get_intervals_at(-238, elem.height) → intervals=[]
  → y += 0.5  // 死区爬行
  → get_intervals_at(-237.5, elem.height) → intervals=[]
  → y += 0.5
  → … （重复 200+ 次）
  → 等到 Y≈-130 时，区间宽度才足够 = 200 个单位
  
  此时 Y 已经从 -238 爬到了 -130，消耗了 108 个单位！
  容器总高 = 340 - (-238) = 578
  可用上半空间只剩：max_y(=340) - (-130) = 470 → 约 81%
  
  但如果有 6 个元素，每行约 50 高 + 间隔 5，6 行 = 330
  470 空间放 6 行 → 够用
  可问题在于：第一个元素的初始行高就是 40-50！
  当 Y 还在 -238~-220 时，行高 40 意味着 get_intervals_at 的 height 参数是 40
  高度 40 的切片在尖端区域 = 全部在容器外 → 空区间 → dead loop
```

**量化估算**：
- 心形底部尖角从 Y=-238 到 Y≈-130 约有 108 个单位高度
- 步长 0.5 → 需要 **216 次循环迭代**才走出死区
- 216 次都调用 `get_intervals_at`（含 ioverlay 布尔运算）——白白消耗 CPU

### 3.2 问题 B：元素全部蜷缩在一侧心房

**现象**：心形上半部分出现左右两个心房（两段可用区间），但所有元素被塞进其中一个

**根因**：

```rust
// engine.rs line 100-109
let widest = row_range
    .intervals
    .iter()
    .max_by(|a, b| (a.1 - a.0).partial_cmp(&(b.1 - b.0)).unwrap_or(…))
    .expect("row_range is non-empty");
```

每次只取**最宽的那个区间**，其余区间全部丢弃。

**心形左右心房宽度对比**（从日志计算）：

| Y 位置 | 左心房 (l, r) | 宽度 | 右心房 (l, r) | 宽度 | winner |
|:---|:---|:---:|:---|:---:|:---|
| -232 | (-200.5, -113.7) | 86.8 | (113.7, 200.5) | 86.8 | 等宽 |
| -190 | (-272.2, -45.0) | 227.3 | (45.0, 272.2) | 227.3 | 等宽 |
| -137 | (-308.2, -9.6) | 298.6 | (9.6, 308.2) | 298.6 | 等宽 |

日志显示对心形而言左右宽度**完全相同**（对称），所以"取最宽"就变成随机的（`.max_by` 遇到相等返回第一个，所以永远是左心房被选中）。

### 3.3 问题 C：step_size 0.5 与行高 40-50 的不匹配

`get_intervals_at(y, row_height=40, min_width)` 在尖角处创建一个 40 单位高的切片矩形。这个切片绝大部分在容器外 → ioverlay AND 结果为空 → `is_empty()` → `y += 0.5`。

**但如果用 `step_size = row_height`**（比如 40），每次就跳过 40 个单位，只需 3 次就能离开死区。代价是可能跳过一些"刚好能放下"的窄行——但在工业排版的 VDM 场景下，一个元素放不下的行就是无意义的。

---

## 四、改进方案

### 4.1 方案总览

| 层级 | 方案 | 解决的问题 | 复杂度 | 风险 |
|:---|:---|:---|:---:|:---|
| P0 | 死区智能跳跃 | A: 上半空白 + C: 无效探测 | 低 | 极低 |
| P1 | 多区间并行 | B: 元素蜷缩单侧 | 中高 | 中 |
| P2 | 精细跳跃（Jump Table） | A 的终极解决方案 | 高 | 低 |

### 4.2 P0：死区智能跳跃（推荐立即执行）

**核心思想**：当区间放不下当前第一个元素时，不要用 `step_size=0.5` 微爬，而是直接跳过有意义的最小步长。

**实现**：

```rust
// 当前代码 (engine.rs line 94-97):
if row_range.is_empty() {
    y += config.step_size;
    continue;
}

// 改为：
if row_range.is_empty() {
    // 智能跳跃：当区间完全不可用时，至少跳过当前元素高度
    let jump = elements[idx].height.max(config.step_size);
    y += jump;
    continue;
}
```

**同理，line 114-118 也应该统一**：

```rust
if interval_r - interval_l <= 0.0 {
    let jump = elements[idx].height.max(config.step_size);
    y += jump;
    continue;
}
```

**效果估算**（心形场景）：
- 改进前：216 次死区迭代 × get_intervals_at
- 改进后：108/40 = **3 次**（元素高度 40 约等于步长）

**为什么安全**：
- 如果给一个元素 `height=40` 都找不到可用区间，那么给 `height=1` 更不可能找到（因为区间宽度受 `min_width` 限制，和 height 无关）
- 跳过一个元素高度不会错过"更高但放不下一行的区域"——因为当前行的首元素都放不下，自然不可能放过后续更高元素

### 4.3 P1：多区间并行（推荐 Phase 3 执行）

**核心思想**：不取"最宽区间"，而是把所有可用区间都用起来——元素可以被分配到不同的心房里。

**两种子方案**：

#### 子方案 A：均匀分配（简单）

```
对每个区间，按区间宽度比例分配元素：
  total_width = Σ(interval_width_i)
  区间 j 获得元素数量 = round(n * interval_width_j / total_width)
  
然后对每个区间独立调用 solve_row_x
```

优点：实现简单，左右心房各一半  
缺点：不感知元素语义——可能把标题和副标题拆到两个心房

#### 子方案 B：完整多行（正确）

```
把每个区间视为独立的一行（同一 Y 起始点）：
  for interval in intervals (按宽度降序):
    pack_row_elements(interval_width) → 该区间装尽可能多的元素
    独立 solve_row_x
  所有区间处理完 → y += max(row_heights) + line_spacing
```

**关键细节**：所有并行区间必须在相同的 Y 起始点，不同区间可以有不同行高，但一起推进 Y 时取最大值。

优点：正确的语义——每个区间独立打包，多少不拘  
缺点：需要修改 `layout_rows` 主循环结构

### 4.4 P2：精细跳跃——Jump Table 预计算

**核心思想**：在 `RangeGenerator` 初始化时预计算"元素可排入的首次 Y 位置表"。

**实现**：

```rust
pub struct RangeGenerator {
    // … 现有字段 …
    
    /// 预计算的跳跃表：每个高度对应的第一个可用 Y 位置
    /// jump_table[h] = 第一个能放下高度 h 的行的 Y 坐标
    jump_table: BTreeMap<usize, f64>,
}

impl RangeGenerator {
    /// 查找第一个能放下给定高度的行的 Y 位置
    pub fn first_available_y(&self, height: f64, min_width: Option<f64>) -> Option<f64> {
        // 在 jump_table 中二分查找
    }
}
```

在主循环中：
```rust
// 替代死区爬行：
let first_y = rg.first_available_y(elements[idx].height, config.min_width);
if let Some(jump_y) = first_y {
    if jump_y > y {
        y = jump_y;  // 一键跳跃
    }
}
```

**代价**：
- 初始化的时间/空间开销（心形 ~500 条离散记录，每条 O(ioverlay AND)，总初始化时间 < 50ms）
- 每改一次容器形状需要重新初始化

**收益**：
- Y 推进变为 O(1) 查找，不再有任何无效探测
- 对于 U 形、W 形等复杂容器，收益更大

---

## 五、可选方案对比

| 维度 | P0 智能跳跃 | P1 多区间并行 | P2 Jump Table |
|:---|:---|:---|:---|
| 解决"上半空白" | ✅ 完全解决 | ✅ 有间接帮助 | ✅ 完美解决 |
| 解决"单侧蜷缩" | ❌ 不涉及 | ✅ 完全解决 | ❌ 不涉及 |
| 实现复杂度 | 2 行改动 | ~80 行新代码 | ~100 行新代码 + 数据结构 |
| 引入新 bug 风险 | 极低 | 中（并行行处理复杂） | 低 |
| 对现有测试影响 | 无 | 需要更新测试 | 无 |
| WASM 兼容 | ✅ | ✅ | ✅ （BTreeMap 是纯 Rust） |
| CPU 开销 | 降低 ~50x | 略微增加（多次调用 solver） | 初始化轻微增加，查询降低 |

---

## 六、建议路线

### 即刻执行

```
Step 1: P0 智能跳跃
  - engine.rs line 96:  step_size → elements[idx].height.max(config.step_size)
  - engine.rs line 116: step_size → elements[idx].height.max(config.step_size)
  - 跑现有 34 个测试 + bevy_visual_demo
  - 预期：上半区域不再空白，元素从顶部开始排
  - 工作量：30 分钟
```

### 第一阶段完成后验证

- 如果 P0 后心形上半部分有元素，下半有元素，且左右心房问题仍存在 → 确认问题 B 是独立的
- 如果 P0 后整体布局视觉可接受 → P1 可以推迟

### Phase 3 执行

```
Step 2: P1 多区间并行
  - 重构 layout_rows 主循环，支持同一 Y 起点多区间
  - 保持现有单区间路径作为 fallback
  - 工作量：4-6 小时
  
Step 3: P2 Jump Table（可选，Phase 4+）
  - 当容器高度极大（>2000 单位）或形状极复杂时
  - P0 已经解决了 99% 的死区问题，P2 是锦上添花
```

---

## 附录：相关代码索引

| 功能 | 文件 | 行号 |
|:---|:---|:---|
| `layout_rows` 主循环 | `src/engine.rs` | 43-303 |
| 死区跳跃 (step_size) | `src/engine.rs` | 96, 116 |
| 行高跳跃 (row_height) | `src/engine.rs` | 126 |
| `get_intervals_at` | `src/region.rs` | 285-360 |
| `find_bottleneck` | `src/region.rs` | 373-431 |
| `LayoutConfig.step_size` | `src/rules.rs` | 52-53 |
| `pack_row_elements` | `src/engine.rs` | 312-378 |
| `solve_row_x` | `src/engine.rs` | 400-… |
| bevy_visual_demo 场景 | `examples/bevy_visual_demo.rs` | 119-199 |
