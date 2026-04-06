use crate::actions::doctor;
use crate::config::Config;
use crate::error::Result;
use crate::output;

/// Run doctor (validate only). Delegates to actions::doctor::scan.
pub fn run(config: &Config, context: Option<&str>, format: &str) -> Result<()> {
    let report = doctor::scan(config, context)?;

    if format == "json" {
        // Use output::plain, NOT println
        output::plain(serde_json::to_string_pretty(&report).unwrap_or_default());
        return Ok(());
    }

    if report.total_issues == 0 {
        output::success(format!(
            "{} files checked — no issues found",
            report.files_checked
        ));
        return Ok(());
    }

    print_issues(&report);
    print_summary(&report);

    Ok(())
}

/// Run doctor fix (validate + auto-fix). Delegates to actions::doctor.
pub fn run_fix(config: &Config, context: Option<&str>, dry_run: bool) -> Result<()> {
    let report = doctor::fix(config, context, dry_run)?;
    let total = report.fields_renamed
        + report.fields_set
        + report.files_renamed
        + report.files_relocated
        + report.manifest_updated
        + report.manifest_removed;

    if dry_run {
        output::dim(format!(
            "Dry run: would apply {total} fixes ({} field renames, {} fields set, {} file renames, {} relocations, {} manifest updates, {} manifest removals)",
            report.fields_renamed,
            report.fields_set,
            report.files_renamed,
            report.files_relocated,
            report.manifest_updated,
            report.manifest_removed
        ));
    } else {
        output::success(format!(
            "Fixed: {} field renames, {} fields set, {} file renames, {} relocations",
            report.fields_renamed, report.fields_set, report.files_renamed, report.files_relocated
        ));
        if report.manifest_updated > 0 || report.manifest_removed > 0 {
            output::dim(format!(
                "Manifest: {} entries updated, {} stale entries removed",
                report.manifest_updated, report.manifest_removed
            ));
        }
    }
    Ok(())
}

fn print_issues(report: &doctor::DoctorReport) {
    for result in &report.file_results {
        if result.issues.is_empty() {
            continue;
        }
        output::header(&result.file_path);
        for issue in &result.issues {
            let fixable_tag = if issue.auto_fixable {
                " [auto-fixable]"
            } else {
                ""
            };
            let path_tag = if issue.path.is_empty() {
                String::new()
            } else {
                format!(" {}", issue.path)
            };
            if issue.auto_fixable {
                output::warning(format!("{path_tag}: {}{fixable_tag}", issue.message));
            } else {
                output::error(format!("{path_tag}: {}", issue.message));
            }
        }
        output::blank();
    }
}

fn print_summary(report: &doctor::DoctorReport) {
    output::label("Checked", report.files_checked);
    output::label(
        "Issues",
        format!(
            "{} ({} auto-fixable, {} manual)",
            report.total_issues,
            report.auto_fixable,
            report.total_issues - report.auto_fixable,
        ),
    );
    if report.auto_fixable > 0 {
        output::hint(
            "Run `temper doctor fix` to auto-fix, or `temper doctor fix --dry-run` to preview.",
        );
    }
}
