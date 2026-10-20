//! Rendering for `comline_core`'s [`SchemaChanges`], shared by `build` and `diff`.

use comline_core::schema::ir::diff::{BreakingChange, Modification, NewFeature, SchemaChanges};
use comline_core::schema::ir::unverified;

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
        BreakingChange::ChangedFieldOptionality {
            type_name,
            field_name,
            now_optional,
        } => {
            let now = if *now_optional {
                "optional"
            } else {
                "required"
            };
            mark(
                "🔄",
                "~",
                format!("Made field `{type_name}.{field_name}` {now}"),
            )
        }
        BreakingChange::RenamedEnumVariant {
            enum_name,
            old_name,
            new_name,
        } => mark(
            "🔄",
            "~",
            format!(
                "Renamed `{enum_name}::{old_name}` {} `{enum_name}::{new_name}`",
                ui::arrow()
            ),
        ),
        BreakingChange::RemovedThrownError {
            protocol_name,
            function_name,
            error,
        } => mark(
            "❌",
            "-",
            format!("`{protocol_name}.{function_name}` no longer declares `! {error}`"),
        ),
        BreakingChange::ChangedErrorOrdinal {
            name,
            old_ordinal,
            new_ordinal,
        } => mark(
            "🔄",
            "~",
            format!(
                "Error `{name}`'s ordinal moved: {old_ordinal} {} {new_ordinal}",
                ui::arrow()
            ),
        ),
        BreakingChange::RemovedConstant { name } => {
            mark("❌", "-", format!("Removed constant `{name}`"))
        }
        BreakingChange::ChangedConstantType {
            name,
            old_type,
            new_type,
        } => mark(
            "🔄",
            "~",
            format!(
                "Changed constant `{name}`: {old_type} {} {new_type}",
                ui::arrow()
            ),
        ),
        BreakingChange::RemovedTypeAlias { name } => {
            mark("❌", "-", format!("Removed type alias `{name}`"))
        }
        BreakingChange::ChangedTypeAliasTarget {
            name,
            old_target,
            new_target,
        } => mark(
            "🔄",
            "~",
            format!(
                "Changed type alias `{name}`: {old_target} {} {new_target}",
                ui::arrow()
            ),
        ),
        BreakingChange::RemovedValidator { name } => {
            mark("❌", "-", format!("Removed validator `{name}`"))
        }
        BreakingChange::ChangedValidator { name } => {
            mark("🔄", "~", format!("Changed validator `{name}`"))
        }
        BreakingChange::ValidationTightened { target } => {
            mark("🔄", "~", format!("Validation tightened on `{target}`"))
        }
        BreakingChange::ChangedWireAnnotation {
            target,
            key,
            old_value,
            new_value,
        } => mark(
            "🔄",
            "~",
            format!(
                "Changed `@{key}` on `{target}`: {}",
                value_change(old_value, new_value)
            ),
        ),
        BreakingChange::GaveUpGuarantees { target, guarantees } => mark(
            "🔓",
            "-",
            format!(
                "`{target}` now gives up {}",
                unverified::describe(guarantees)
            ),
        ),
    }
}

/// `old → new`, with `(none)` for a side that has no value.
fn value_change(old: &Option<String>, new: &Option<String>) -> String {
    let show = |v: &Option<String>| v.clone().unwrap_or_else(|| "(none)".to_string());
    format!("{} {} {}", show(old), ui::arrow(), show(new))
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
        NewFeature::AddedThrownError {
            protocol_name,
            function_name,
            error,
        } => mark(
            "➕",
            "+",
            format!("`{protocol_name}.{function_name}` now declares `! {error}`"),
        ),
        NewFeature::AddedConstant { name, ty } => {
            mark("➕", "+", format!("Added constant `{name}`: {ty}"))
        }
        NewFeature::ChangedConstantValue {
            name,
            old_value,
            new_value,
        } => mark(
            "🔧",
            "~",
            format!(
                "Changed constant `{name}`'s value: {}",
                value_change(old_value, new_value)
            ),
        ),
        NewFeature::AddedTypeAlias { name, target } => {
            mark("➕", "+", format!("Added type alias `{name}` = {target}"))
        }
        NewFeature::AddedValidator { name } => mark("➕", "+", format!("Added validator `{name}`")),
        NewFeature::ValidationLoosened { target } => {
            mark("🔧", "~", format!("Validation loosened on `{target}`"))
        }
        NewFeature::Deprecated { target } => mark("⚠️", "!", format!("Deprecated `{target}`")),
        NewFeature::RestoredGuarantees { target, guarantees } => mark(
            "🔒",
            "+",
            format!(
                "`{target}` keeps {} again",
                unverified::describe(guarantees)
            ),
        ),
    }
}

fn modification_line(modification: &Modification) -> String {
    match modification {
        Modification::DocstringChanged { name } => {
            mark("📝", "~", format!("Docstring changed on `{name}`"))
        }
        Modification::ChangedFieldDefault {
            type_name,
            field_name,
        } => mark(
            "🔧",
            "~",
            format!("Changed the default of `{type_name}.{field_name}`"),
        ),
        Modification::ChangedErrorMessage { name } => {
            mark("📝", "~", format!("Changed error `{name}`'s message"))
        }
        Modification::ChangedAnnotation {
            target,
            key,
            old_value,
            new_value,
        } => mark(
            "🔧",
            "~",
            format!(
                "Changed `@{key}` on `{target}`: {}",
                value_change(old_value, new_value)
            ),
        ),
    }
}
