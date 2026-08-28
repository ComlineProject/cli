//! Rendering for `comline_core`'s [`SchemaChanges`], shared by `build` and `diff`.

use comline_core::schema::ir::diff::{BreakingChange, Modification, NewFeature, SchemaChanges};

use crate::ui;

/// Print a grouped breaking / feature / modification changelog.
pub fn render(changes: &SchemaChanges) {
    if !changes.breaking_changes.is_empty() {
        ui::group(heading(
            "🔴",
            "Breaking changes",
            changes.breaking_changes.len(),
        ));
        for change in &changes.breaking_changes {
            ui::detail(breaking_line(change));
        }
    }

    if !changes.new_features.is_empty() {
        ui::group(heading("🟢", "New features", changes.new_features.len()));
        for feature in &changes.new_features {
            ui::detail(feature_line(feature));
        }
    }

    if !changes.modifications.is_empty() {
        ui::group(heading("🔵", "Modifications", changes.modifications.len()));
        for modification in &changes.modifications {
            ui::detail(modification_line(modification));
        }
    }

    if changes.is_empty() {
        ui::detail("no schema changes");
    }
}

fn heading(emoji: &str, text: &str, n: usize) -> String {
    if ui::plain() {
        format!("{text} ({n}):")
    } else {
        format!("{emoji} {text} ({n})")
    }
}

/// Prefix `body` with a marker: an emoji, or its ASCII stand-in under `--plain`.
fn mark(emoji: &str, ascii: &str, body: String) -> String {
    let m = if ui::plain() { ascii } else { emoji };
    format!("{m} {body}")
}

fn count(n: usize, singular: &str) -> String {
    if n == 1 {
        format!("{n} {singular}")
    } else {
        format!("{n} {singular}s")
    }
}

fn breaking_line(change: &BreakingChange) -> String {
    match change {
        BreakingChange::RemovedStruct { name } => {
            mark("❌", "-", format!("Removed struct `{name}`"))
        }
        BreakingChange::RemovedEnum { name } => mark("❌", "-", format!("Removed enum `{name}`")),
        BreakingChange::RemovedField {
            type_name,
            field_name,
        } => mark(
            "❌",
            "-",
            format!("Removed field `{type_name}.{field_name}`"),
        ),
        BreakingChange::ChangedFieldType {
            type_name,
            field_name,
            old_type,
            new_type,
        } => mark(
            "🔄",
            "~",
            format!(
                "Changed `{type_name}.{field_name}`: {old_type} {} {new_type}",
                ui::arrow()
            ),
        ),
        BreakingChange::AddedRequiredField {
            type_name,
            field_name,
            field_type,
        } => mark(
            "⚠️",
            "!",
            format!("Added required field `{type_name}.{field_name}`: {field_type}"),
        ),
        BreakingChange::RemovedEnumVariant { enum_name, variant } => {
            mark("❌", "-", format!("Removed `{enum_name}::{variant}`"))
        }
        BreakingChange::RemovedFunction {
            protocol_name,
            function_name,
        } => mark(
            "❌",
            "-",
            format!("Removed function `{protocol_name}.{function_name}`"),
        ),
        BreakingChange::ChangedFunctionSignature {
            protocol_name,
            function_name,
            details,
        } => mark(
            "🔄",
            "~",
            format!("Changed signature of `{protocol_name}.{function_name}`: {details}"),
        ),
        BreakingChange::RemovedProtocol { name } => {
            mark("❌", "-", format!("Removed protocol `{name}`"))
        }
        BreakingChange::RemovedError { name } => mark("❌", "-", format!("Removed error `{name}`")),
    }
}

fn feature_line(feature: &NewFeature) -> String {
    match feature {
        NewFeature::AddedStruct { name, field_count } => mark(
            "➕",
            "+",
            format!("Added struct `{name}` ({})", count(*field_count, "field")),
        ),
        NewFeature::AddedEnum {
            name,
            variant_count,
        } => mark(
            "➕",
            "+",
            format!("Added enum `{name}` ({})", count(*variant_count, "variant")),
        ),
        NewFeature::AddedField {
            type_name,
            field_name,
            field_type,
            optional,
        } => {
            let opt = if *optional { " (optional)" } else { "" };
            mark(
                "➕",
                "+",
                format!("Added field `{type_name}.{field_name}`: {field_type}{opt}"),
            )
        }
        NewFeature::AddedEnumVariant { enum_name, variant } => {
            mark("➕", "+", format!("Added variant `{enum_name}::{variant}`"))
        }
        NewFeature::AddedFunction {
            protocol_name,
            function_name,
            signature,
        } => mark(
            "➕",
            "+",
            format!("Added function `{protocol_name}.{function_name}`: {signature}"),
        ),
        NewFeature::AddedProtocol {
            name,
            function_count,
        } => mark(
            "➕",
            "+",
            format!(
                "Added protocol `{name}` ({})",
                count(*function_count, "function")
            ),
        ),
        NewFeature::AddedError { name, field_count } => mark(
            "➕",
            "+",
            format!("Added error `{name}` ({})", count(*field_count, "field")),
        ),
    }
}

fn modification_line(modification: &Modification) -> String {
    match modification {
        Modification::FieldMadeOptional {
            type_name,
            field_name,
        } => mark(
            "🔧",
            "~",
            format!("Made field `{type_name}.{field_name}` optional"),
        ),
        Modification::DocstringChanged { name } => {
            mark("📝", "~", format!("Docstring changed on `{name}`"))
        }
    }
}
