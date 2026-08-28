//! Rendering for `comline_core`'s [`SchemaChanges`], shared by `build` and `diff`.

use comline_core::schema::ir::diff::{BreakingChange, Modification, NewFeature, SchemaChanges};

use crate::ui;

/// Print a grouped breaking / feature / modification changelog.
pub fn render(changes: &SchemaChanges) {
    if !changes.breaking_changes.is_empty() {
        ui::group(format!(
            "🔴 Breaking changes ({})",
            changes.breaking_changes.len()
        ));
        for change in &changes.breaking_changes {
            ui::detail(breaking_line(change));
        }
    }

    if !changes.new_features.is_empty() {
        ui::group(format!("🟢 New features ({})", changes.new_features.len()));
        for feature in &changes.new_features {
            ui::detail(feature_line(feature));
        }
    }

    if !changes.modifications.is_empty() {
        ui::group(format!(
            "🔵 Modifications ({})",
            changes.modifications.len()
        ));
        for modification in &changes.modifications {
            ui::detail(modification_line(modification));
        }
    }

    if changes.is_empty() {
        ui::detail("no schema changes");
    }
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
        BreakingChange::RemovedStruct { name } => format!("❌ Removed struct `{name}`"),
        BreakingChange::RemovedEnum { name } => format!("❌ Removed enum `{name}`"),
        BreakingChange::RemovedField {
            type_name,
            field_name,
        } => format!("❌ Removed field `{type_name}.{field_name}`"),
        BreakingChange::ChangedFieldType {
            type_name,
            field_name,
            old_type,
            new_type,
        } => format!("🔄 Changed `{type_name}.{field_name}`: {old_type} → {new_type}"),
        BreakingChange::AddedRequiredField {
            type_name,
            field_name,
            field_type,
        } => format!("⚠️  Added required field `{type_name}.{field_name}`: {field_type}"),
        BreakingChange::RemovedEnumVariant { enum_name, variant } => {
            format!("❌ Removed `{enum_name}::{variant}`")
        }
        BreakingChange::RemovedFunction {
            protocol_name,
            function_name,
        } => format!("❌ Removed function `{protocol_name}.{function_name}`"),
        BreakingChange::ChangedFunctionSignature {
            protocol_name,
            function_name,
            details,
        } => format!("🔄 Changed signature of `{protocol_name}.{function_name}`: {details}"),
        BreakingChange::RemovedProtocol { name } => format!("❌ Removed protocol `{name}`"),
        BreakingChange::RemovedError { name } => format!("❌ Removed error `{name}`"),
    }
}

fn feature_line(feature: &NewFeature) -> String {
    match feature {
        NewFeature::AddedStruct { name, field_count } => {
            format!(
                "➕ Added struct `{name}` ({})",
                count(*field_count, "field")
            )
        }
        NewFeature::AddedEnum {
            name,
            variant_count,
        } => format!(
            "➕ Added enum `{name}` ({})",
            count(*variant_count, "variant")
        ),
        NewFeature::AddedField {
            type_name,
            field_name,
            field_type,
            optional,
        } => {
            let opt = if *optional { " (optional)" } else { "" };
            format!("➕ Added field `{type_name}.{field_name}`: {field_type}{opt}")
        }
        NewFeature::AddedEnumVariant { enum_name, variant } => {
            format!("➕ Added variant `{enum_name}::{variant}`")
        }
        NewFeature::AddedFunction {
            protocol_name,
            function_name,
            signature,
        } => format!("➕ Added function `{protocol_name}.{function_name}`: {signature}"),
        NewFeature::AddedProtocol {
            name,
            function_count,
        } => format!(
            "➕ Added protocol `{name}` ({})",
            count(*function_count, "function")
        ),
        NewFeature::AddedError { name, field_count } => {
            format!("➕ Added error `{name}` ({})", count(*field_count, "field"))
        }
    }
}

fn modification_line(modification: &Modification) -> String {
    match modification {
        Modification::FieldMadeOptional {
            type_name,
            field_name,
        } => format!("🔧 Made field `{type_name}.{field_name}` optional"),
        Modification::DocstringChanged { name } => format!("📝 Docstring changed on `{name}`"),
    }
}
