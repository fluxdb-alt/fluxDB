// 结构差异视图：把两端整表定义按行比对，用 git 风格（- / + / 上下文）展示。
//
// 只做展示，不重新比对结构：两端文本由控制器按目标方言渲染好，这里只负责
// 逐行对齐。行数通常只有几十行，直接上 O(n*m) 的 LCS，不需要额外依赖。

/// 结构预览里单表的状态分类，驱动左侧徽标与顶部统计。
#[derive(Clone, Copy, Eq, PartialEq)]
enum StructureTableKind {
    Create,
    Alter,
    Drop,
    Equal,
    Skipped,
}

/// 按变更内容归类：整表级 Create/Drop 才算新增/删除，其余差异都算修改。
fn structure_table_kind(table: &TableStructureDiff) -> StructureTableKind {
    if table.skip.is_some() || table.error.is_some() {
        return StructureTableKind::Skipped;
    }
    if table.state == TableStructureState::Equal || table.changes.is_empty() {
        return StructureTableKind::Equal;
    }
    let all_table_level = |action: StructureChangeAction| {
        table
            .changes
            .iter()
            .all(|change| change.object == StructureObject::Table && change.action == action)
    };
    if all_table_level(StructureChangeAction::Create) {
        StructureTableKind::Create
    } else if all_table_level(StructureChangeAction::Drop) {
        StructureTableKind::Drop
    } else {
        StructureTableKind::Alter
    }
}

fn structure_state_label(kind: StructureTableKind) -> SharedString {
    match kind {
        StructureTableKind::Create => tr("app.sync.change_create"),
        StructureTableKind::Alter => tr("app.sync.change_alter"),
        StructureTableKind::Drop => tr("app.sync.change_drop"),
        StructureTableKind::Equal => tr("app.sync.same"),
        StructureTableKind::Skipped => tr("app.sync.structure_skipped"),
    }
}

fn structure_state_color(kind: StructureTableKind, colors: UiColors) -> gpui::Hsla {
    match kind {
        StructureTableKind::Create => colors.success.into(),
        StructureTableKind::Alter => colors.warning.into(),
        StructureTableKind::Drop => colors.danger.into(),
        StructureTableKind::Equal => colors.muted.into(),
        StructureTableKind::Skipped => colors.muted.into(),
    }
}

/// 状态徽标：淡色底 + 状态色文字，和差异视图的 -/+ 配色一致。
fn structure_state_chip(kind: StructureTableKind, colors: UiColors) -> Div {
    let color = structure_state_color(kind, colors);
    div()
        .px_1p5()
        .py_0p5()
        .rounded(colors.radius)
        .bg(color.alpha(0.12))
        .text_size(ui_text_size(10.))
        .text_color(color)
        .child(structure_state_label(kind))
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum DiffLineKind {
    Context,
    Removed,
    Added,
}

/// 对齐后的一行：`old_no` 对应目标端现状，`new_no` 对应源端目标态。
struct DiffLine {
    old_no: Option<usize>,
    new_no: Option<usize>,
    kind: DiffLineKind,
    text: String,
}

/// 行级 LCS 差异。两侧完全一致时全部输出为上下文行。
fn diff_lines(old: &str, new: &str) -> Vec<DiffLine> {
    let old: Vec<&str> = old.lines().collect();
    let new: Vec<&str> = new.lines().collect();
    // lcs[i][j] = old[i..] 与 new[j..] 的最长公共子序列长度。
    let mut lcs = vec![vec![0u32; new.len() + 1]; old.len() + 1];
    for i in (0..old.len()).rev() {
        for j in (0..new.len()).rev() {
            lcs[i][j] = if old[i] == new[j] {
                lcs[i + 1][j + 1] + 1
            } else {
                lcs[i + 1][j].max(lcs[i][j + 1])
            };
        }
    }
    let mut lines = Vec::with_capacity(old.len().max(new.len()));
    let (mut i, mut j) = (0, 0);
    while i < old.len() || j < new.len() {
        // 优先对齐相同的行；否则先输出删除再输出新增，保持 git 的阅读顺序。
        if i < old.len() && j < new.len() && old[i] == new[j] {
            lines.push(DiffLine {
                old_no: Some(i + 1),
                new_no: Some(j + 1),
                kind: DiffLineKind::Context,
                text: old[i].to_string(),
            });
            i += 1;
            j += 1;
        } else if i < old.len() && (j >= new.len() || lcs[i + 1][j] >= lcs[i][j + 1]) {
            lines.push(DiffLine {
                old_no: Some(i + 1),
                new_no: None,
                kind: DiffLineKind::Removed,
                text: old[i].to_string(),
            });
            i += 1;
        } else {
            lines.push(DiffLine {
                old_no: None,
                new_no: Some(j + 1),
                kind: DiffLineKind::Added,
                text: new[j].to_string(),
            });
            j += 1;
        }
    }
    lines
}

/// 差异布局切换：统一视图（上下对照）与并排视图（左右对照）共用同一份行差异。
fn structure_diff_layout_tab(
    split: bool,
    label: SharedString,
    active: bool,
    colors: UiColors,
    cx: &mut Context<NavicatMain>,
) -> Stateful<Div> {
    div()
        .debug_selector(move || {
            format!(
                "sync-diff-layout:{}",
                if split { "split" } else { "unified" }
            )
        })
        .id(("sync-diff-layout", u64::from(split)))
        .px_2()
        .py_0p5()
        .rounded(colors.radius)
        .text_size(ui_text_size(10.5))
        .text_color(if active { colors.text } else { colors.muted })
        .bg(if active { colors.tree_selected } else { colors.hover })
        .cursor_pointer()
        .on_click(cx.listener(move |this, _, _, cx| {
            if let Some(state) = this.sync_wizard.as_mut() {
                state.structure_diff_split = split;
            }
            cx.notify();
        }))
        .child(label)
}

/// 整表定义的 git 风格差异：左侧行号是目标端现状，右侧是源端目标态。
fn structure_ddl_diff_view(current: &str, desired: &str, split: bool, colors: UiColors) -> Div {
    if split {
        split_diff_view(current, desired, colors)
    } else {
        unified_diff_view(current, desired, colors)
    }
}

fn unified_diff_view(current: &str, desired: &str, colors: UiColors) -> Div {
    let lines = diff_lines(current, desired);
    let has_changes = lines.iter().any(|line| line.kind != DiffLineKind::Context);
    div()
        .debug_selector(|| "sync-structure-diff".into())
        .rounded(colors.radius)
        .border_1()
        .border_color(colors.border_soft)
        .bg(colors.panel_alt)
        .py_1()
        .flex()
        .flex_col()
        .font_family(crate::PLATFORM_MONOSPACE_FONT)
        .text_size(ui_text_size(10.5))
        .when(!has_changes, |this| {
            this.child(
                div()
                    .px_3()
                    .py_1()
                    .text_color(colors.muted)
                    .child(tr("app.sync.same")),
            )
        })
        .children(lines.into_iter().map(|line| diff_line_row(line, colors)))
}

fn diff_line_row(line: DiffLine, colors: UiColors) -> Div {
    let (bg, text_color, marker) = match line.kind {
        DiffLineKind::Context => (None, colors.text, ""),
        DiffLineKind::Removed => (Some(colors.danger.alpha(0.12)), colors.danger, "-"),
        DiffLineKind::Added => (Some(colors.success.alpha(0.12)), colors.success, "+"),
    };
    div()
        .h(px(20.))
        .flex()
        .items_center()
        .when_some(bg, |this, bg| this.bg(bg))
        .child(diff_gutter(line.old_no, colors))
        .child(diff_gutter(line.new_no, colors))
        .child(
            div()
                .w(px(14.))
                .flex_none()
                .text_color(text_color)
                .child(marker),
        )
        .child(
            div()
                .flex_1()
                .min_w(px(0.))
                .truncate()
                .text_color(text_color)
                .child(line.text),
        )
}

fn diff_gutter(number: Option<usize>, colors: UiColors) -> Div {
    div()
        .w(px(38.))
        .flex_none()
        .pr_2()
        .text_right()
        .text_color(colors.muted.opacity(0.65))
        .child(number.map(|number| number.to_string()).unwrap_or_default())
}

/// 并排视图：左列=目标端现状，右列=源端目标态；行按 diff 对齐，空位留白。
fn split_diff_view(current: &str, desired: &str, colors: UiColors) -> Div {
    let lines = diff_lines(current, desired);
    div()
        .debug_selector(|| "sync-structure-diff".into())
        .rounded(colors.radius)
        .border_1()
        .border_color(colors.border_soft)
        .bg(colors.panel_alt)
        .py_1()
        .flex()
        .flex_col()
        .font_family(crate::PLATFORM_MONOSPACE_FONT)
        .text_size(ui_text_size(10.5))
        .child(
            div()
                .h(px(22.))
                .flex()
                .items_center()
                .text_color(colors.muted)
                .child(split_header_cell(tr("app.sync.diff_current")))
                .child(div().w(px(1.)).h_full().bg(colors.border_soft))
                .child(split_header_cell(tr("app.sync.diff_desired"))),
        )
        .children(lines.into_iter().map(|line| split_row(line, colors)))
}

fn split_header_cell(label: SharedString) -> Div {
    div().flex_1().min_w(px(0.)).px_2().truncate().child(label)
}

fn split_row(line: DiffLine, colors: UiColors) -> Div {
    let removed = line.kind == DiffLineKind::Removed;
    let added = line.kind == DiffLineKind::Added;
    div()
        .h(px(20.))
        .flex()
        .items_center()
        .child(split_cell(
            if added { None } else { line.old_no },
            (!added).then(|| line.text.clone()),
            removed.then_some(colors.danger),
            colors,
        ))
        .child(div().w(px(1.)).h_full().bg(colors.border_soft))
        .child(split_cell(
            if removed { None } else { line.new_no },
            (!removed).then(|| line.text.clone()),
            added.then_some(colors.success),
            colors,
        ))
}

/// 并排视图的单侧单元格：行号 + 文本，变更侧带底色，空位只留底色/留白。
fn split_cell(
    number: Option<usize>,
    text: Option<String>,
    tint: Option<gpui::Rgba>,
    colors: UiColors,
) -> Div {
    let text_color = tint.unwrap_or(colors.text);
    div()
        .flex_1()
        .min_w(px(0.))
        .h_full()
        .flex()
        .items_center()
        .when_some(tint, |this, tint| this.bg(tint.alpha(0.12)))
        .child(diff_gutter(number, colors))
        .child(
            div()
                .flex_1()
                .min_w(px(0.))
                .truncate()
                .text_color(text_color)
                .child(text.unwrap_or_default()),
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn render(lines: &[DiffLine]) -> Vec<String> {
        lines
            .iter()
            .map(|line| {
                let marker = match line.kind {
                    DiffLineKind::Context => " ",
                    DiffLineKind::Removed => "-",
                    DiffLineKind::Added => "+",
                };
                format!("{marker}{}", line.text)
            })
            .collect()
    }

    #[test]
    fn diff_marks_changed_lines_only() {
        let old = "CREATE TABLE t (\n  `a` int NOT NULL,\n  `b` int NOT NULL\n);";
        let new = "CREATE TABLE t (\n  `a` int NOT NULL,\n  `b` varchar(20) NOT NULL,\n  `c` int\n);";
        assert_eq!(
            render(&diff_lines(old, new)),
            vec![
                " CREATE TABLE t (",
                "   `a` int NOT NULL,",
                "-  `b` int NOT NULL",
                "+  `b` varchar(20) NOT NULL,",
                "+  `c` int",
                " );",
            ]
        );
    }

    #[test]
    fn diff_keeps_line_numbers_per_side() {
        let lines = diff_lines("a\nb", "a\nc");
        assert_eq!(lines[0].old_no, Some(1));
        assert_eq!(lines[0].new_no, Some(1));
        assert_eq!(lines[1].old_no, Some(2));
        assert_eq!(lines[1].new_no, None);
        assert_eq!(lines[2].old_no, None);
        assert_eq!(lines[2].new_no, Some(2));
    }

    #[test]
    fn diff_of_empty_current_is_all_added() {
        let lines = diff_lines("", "CREATE TABLE t (\n  `id` int\n);");
        assert!(lines.iter().all(|line| line.kind == DiffLineKind::Added));
        assert_eq!(lines.len(), 3);
    }
}
