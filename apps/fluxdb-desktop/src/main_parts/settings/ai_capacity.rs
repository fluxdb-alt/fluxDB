// 模型容量单位及自动填充：只接受整数；显示值按 K/M 向下取整。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AiTokenUnit {
    K,
    M,
}
impl AiTokenUnit {
    fn from_label(label: &str) -> Self {
        if label == "M" { Self::M } else { Self::K }
    }
    fn label(self) -> &'static str {
        match self {
            Self::K => "K",
            Self::M => "M",
        }
    }
    fn scale(self) -> i64 {
        match self {
            Self::K => 1000,
            Self::M => 1_000_000,
        }
    }
}

fn ai_parse_token_count(raw: &str, unit: AiTokenUnit) -> Result<Option<i64>, String> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Ok(None);
    }
    if !raw.bytes().all(|c| c.is_ascii_digit()) {
        return Err(tr("app.ai.capacity.positive").to_string());
    }
    let whole = raw
        .parse::<i64>()
        .map_err(|_| tr("app.ai.capacity.out_of_range").to_string())?;
    let value = whole
        .checked_mul(unit.scale())
        .filter(|v| (1..=1_000_000_000).contains(v))
        .ok_or_else(|| tr("app.ai.capacity.bounds").to_string())?;
    Ok(Some(value))
}

fn ai_format_token_count(tokens: i64, unit: AiTokenUnit) -> String {
    (tokens / unit.scale()).to_string()
}

/// 提案容量按既有 K/M 控件展示，未改动时保留精确 Token 值，避免 8192 被保存成 8000。
fn ai_parse_proposed_token_count(
    raw: &str,
    unit: AiTokenUnit,
    proposed: Option<i64>,
) -> Result<Option<i64>, String> {
    if let Some(exact) = proposed
        && raw.trim() == ai_format_token_count(exact, unit)
    {
        return Ok(Some(exact));
    }
    ai_parse_token_count(raw, unit)
}

/// 回填容量字段时展示的单位：≥1M 用 M，否则用 K，显示值向下取整。
///
/// 单位只是展示单位、不落库，因此必须由 token 数推导 —— 否则会出现「存的是 1M、
/// 重开却显示 1000 K」这种数字与单位不一致的假象。切单位的换算行为保持不变。
fn ai_capacity_unit_for(tokens: i64) -> AiTokenUnit {
    if tokens.abs() >= AiTokenUnit::M.scale() {
        AiTokenUnit::M
    } else {
        AiTokenUnit::K
    }
}

/// 回填单个容量字段：单位下拉与输入框数字来自同一个 token 值。
#[allow(clippy::too_many_arguments)]
fn fill_ai_capacity_field(
    tokens: Option<i64>,
    input: &Entity<InputState>,
    unit_select: &Entity<SelectState<SearchableVec<String>>>,
    unit: &mut AiTokenUnit,
    window: &mut Window,
    cx: &mut Context<NavicatMain>,
) {
    let kind = tokens.map(ai_capacity_unit_for).unwrap_or(AiTokenUnit::K);
    unit_select.update(cx, |select, cx| {
        select.set_selected_value(&kind.label().to_string(), window, cx)
    });
    *unit = kind;
    let text = tokens
        .map(|value| ai_format_token_count(value, kind))
        .unwrap_or_default();
    input.update(cx, |state, cx| state.set_value(text, window, cx));
}

fn ai_capacity_field(
    label: impl Into<SharedString>,
    input: &Entity<InputState>,
    unit: &Entity<SelectState<SearchableVec<String>>>,
    colors: UiColors,
) -> impl IntoElement {
    let label = label.into();
    let field_label = label.clone();
    let unit_label = label.clone();
    div()
        .flex()
        .flex_col()
        .gap_1()
        .min_w(px(0.))
        .child(
            div()
                .text_size(ui_text_size(11.))
                .text_color(colors.text)
                .child(label),
        )
        .child(
            div()
                .flex()
                .gap_1()
                .items_center()
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.))
                        .debug_selector(move || format!("ai-field:{field_label}"))
                        .child(Input::new(input).text_size(ui_text_size(13.))),
                )
                .child(
                    div()
                        .w(px(70.))
                        .flex_none()
                        .debug_selector(move || format!("ai-capacity-unit:{unit_label}"))
                        .child(Select::new(unit).w_full().h(px(32.))),
                ),
        )
}

impl NavicatMain {
    fn change_ai_token_unit(
        &mut self,
        index: usize,
        next: AiTokenUnit,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(dialog) = self.ai_model_dialog.as_mut() else {
            return;
        };
        let previous = dialog.capacity_units[index];
        if previous == next {
            return;
        }
        let input = if index == 0 {
            &dialog.inputs.context_window
        } else {
            &dialog.inputs.max_output
        };
        match ai_parse_token_count(&ai_input_text(input, cx), previous) {
            Ok(tokens) => {
                let text = tokens
                    .map(|n| ai_format_token_count(n, next))
                    .unwrap_or_default();
                if dialog.capacity_autofill[index].as_deref()
                    == Some(ai_input_text(input, cx).as_str())
                {
                    dialog.capacity_autofill[index] = Some(text.clone());
                }
                input.update(cx, |state, cx| state.set_value(text, window, cx));
                dialog.capacity_units[index] = next;
            }
            Err(message) => {
                let select = if index == 0 {
                    &dialog.inputs.context_unit
                } else {
                    &dialog.inputs.output_unit
                };
                select.update(cx, |state, cx| {
                    state.set_selected_value(&previous.label().to_string(), window, cx)
                });
                dialog.validation_error = Some(message);
            }
        }
        cx.notify();
    }

    fn apply_ai_model_capacity(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(dialog) = self.ai_model_dialog.as_mut() else {
            return;
        };
        let selected = ai_input_text(&dialog.inputs.model_key, cx);
        let metadata = dialog
            .model_capabilities
            .iter()
            .find(|m| m.id == selected)
            .cloned();
        for (index, input) in [&dialog.inputs.context_window, &dialog.inputs.max_output]
            .into_iter()
            .enumerate()
        {
            let current = ai_input_text(input, cx);
            // 手动输入优先；切模型时只替换仍未被编辑的自动值。
            if !current.is_empty()
                && dialog.capacity_autofill[index].as_deref() != Some(current.as_str())
            {
                continue;
            }
            let tokens = metadata.as_ref().and_then(|m| {
                if index == 0 {
                    m.context_window_tokens
                } else {
                    m.max_output_tokens
                }
            });
            let kind = tokens.map(ai_capacity_unit_for);
            let text = tokens
                .map(|n| ai_format_token_count(n, kind.unwrap_or(AiTokenUnit::K)))
                .unwrap_or_default();
            if let Some(kind) = kind {
                let select = if index == 0 {
                    &dialog.inputs.context_unit
                } else {
                    &dialog.inputs.output_unit
                };
                select.update(cx, |select, cx| {
                    select.set_selected_value(&kind.label().to_string(), window, cx)
                });
                dialog.capacity_units[index] = kind;
            }
            if current != text {
                input.update(cx, |state, cx| state.set_value(text.clone(), window, cx));
            }
            dialog.capacity_autofill[index] = tokens.map(|_| text);
        }
        cx.notify();
    }
}

#[cfg(test)]
mod ai_capacity_tests {
    use super::*;

    #[test]
    fn formatting_never_produces_decimals() {
        let cases = [
            (131_072, AiTokenUnit::K, "131"),
            (128_000, AiTokenUnit::K, "128"),
            (1_000_000, AiTokenUnit::M, "1"),
            (8_192, AiTokenUnit::K, "8"),
        ];
        for (tokens, unit, expected) in cases {
            assert_eq!(ai_format_token_count(tokens, unit), expected);
        }
        assert_eq!(ai_format_token_count(131_999, AiTokenUnit::K), "131");
    }

    #[test]
    fn integer_units_multiply_by_their_scale() {
        assert_eq!(
            ai_parse_token_count("131", AiTokenUnit::K),
            Ok(Some(131_000)),
        );
        assert_eq!(
            ai_parse_token_count("128", AiTokenUnit::K),
            Ok(Some(128_000)),
        );
        assert!(ai_parse_token_count("1.5", AiTokenUnit::M).is_err());
    }

    #[test]
    fn proposed_capacity_preserves_exact_tokens_until_edited() {
        assert_eq!(
            ai_parse_proposed_token_count("8", AiTokenUnit::K, Some(8192)),
            Ok(Some(8192))
        );
        assert_eq!(
            ai_parse_proposed_token_count("9", AiTokenUnit::K, Some(8192)),
            Ok(Some(9000))
        );
        assert_eq!(
            ai_parse_proposed_token_count("", AiTokenUnit::K, Some(8192)),
            Ok(None)
        );
    }

    #[test]
    fn auto_unit_uses_m_for_million_scale_and_k_below_it() {
        assert_eq!(ai_capacity_unit_for(1_000_000), AiTokenUnit::M);
        assert_eq!(ai_capacity_unit_for(200_000), AiTokenUnit::K);
        assert_eq!(ai_capacity_unit_for(131_072), AiTokenUnit::K);
    }
}
