//! Machine-readable descriptions of Graduate command contracts.

use std::io::Write;

use serde_json::json;

use crate::cli::DescribeRestackArgs;
use crate::error::CliError;

pub(crate) fn restack(args: &DescribeRestackArgs) -> Result<(), CliError> {
    if args.json {
        write_restack_schema()?;
    }
    Ok(())
}

pub(crate) fn restack_schema() -> Result<(), CliError> {
    write_restack_schema()
}

fn write_restack_schema() -> Result<(), CliError> {
    let description = json!({
            "kind": "commandDescription",
            "schemaVersion": 1,
            "command": "gd restack",
            "summary": "Review and safely publish an isolated environment reconstruction",
            "arguments": [
                {
                    "name": "environment",
                    "location": "positional",
                    "required": true,
                    "schema": {"type": "string", "format": "git-ref-component"}
                },
                {
                    "name": "main",
                    "location": "option",
                    "flag": "--main",
                    "required": false,
                    "schema": {"type": "string", "format": "git-ref-component"}
                },
                {
                    "name": "remote",
                    "location": "option",
                    "flag": "--remote",
                    "required": false,
                    "default": "origin",
                    "schema": {"type": "string", "format": "git-ref-component"}
                },
                {
                    "name": "params",
                    "location": "option",
                    "flag": "--params",
                    "required": false,
                    "encoding": "inline-json",
                    "conflictsWith": ["resume"],
                    "schemas": ["restackPreviewParams", "restackApplyParams"]
                },
                {
                    "name": "dryRun",
                    "location": "flag",
                    "flag": "--dry-run",
                    "required": false,
                    "schema": {"type": "boolean"},
                    "defaultSelection": {"removeBranches": []},
                    "conflictsWith": ["apply", "resume", "abort"]
                },
                {
                    "name": "apply",
                    "location": "flag",
                    "flag": "--apply",
                    "required": false,
                    "schema": {"type": "boolean"},
                    "conflictsWith": ["dryRun", "abort"]
                },
                {
                    "name": "resume",
                    "location": "option",
                    "flag": "--resume",
                    "required": false,
                    "secret": true,
                    "schema": {"type": "string", "format": "opaque-capability"},
                    "conflictsWith": ["params", "dryRun"]
                },
                {
                    "name": "abort",
                    "location": "flag",
                    "flag": "--abort",
                    "required": false,
                    "schema": {"type": "boolean"},
                    "requires": ["resume"],
                    "conflictsWith": ["dryRun", "apply"]
                }
            ],
            "payloadSchemas": {
                "restackPreviewParams": {
                    "$schema": "https://json-schema.org/draft/2020-12/schema",
                    "type": "object",
                    "additionalProperties": false,
                    "required": ["removeBranches"],
                    "properties": {
                        "removeBranches": {
                            "type": "array",
                            "uniqueItems": true,
                            "items": {"type": "string", "format": "git-ref-component"}
                        }
                    }
                },
                "restackApplyParams": {
                    "$schema": "https://json-schema.org/draft/2020-12/schema",
                    "type": "object",
                    "additionalProperties": false,
                    "required": ["removeBranches", "planDigest"],
                    "properties": {
                        "removeBranches": {
                            "type": "array",
                            "uniqueItems": true,
                            "items": {"type": "string", "format": "git-ref-component"}
                        },
                        "planDigest": {
                            "type": "string",
                            "pattern": "^[0-9a-f]{64}$"
                        }
                    }
                }
            },
            "modes": [
                {
                    "name": "interactive",
                    "selector": "terminal stdin and stderr without --params, --dry-run, or --resume",
                    "mutatesRemote": "only after explicit confirmation"
                },
                {
                    "name": "preview",
                    "selector": "--dry-run, or --params without --apply",
                    "stdoutKind": "restackPlan",
                    "mutatesRemote": false
                },
                {
                    "name": "apply",
                    "selector": "--params with --apply and a reviewed planDigest",
                    "stdoutKind": "restackResult",
                    "mutatesRemote": true
                },
                {
                    "name": "resumePreview",
                    "selector": "--resume without --apply or --abort",
                    "stdoutKind": "restackPlan",
                    "mutatesRemote": false
                },
                {
                    "name": "resumeApply",
                    "selector": "--resume with --apply",
                    "stdoutKind": "restackResult",
                    "mutatesRemote": true
                },
                {
                    "name": "abort",
                    "selector": "--resume with --abort",
                    "stdoutKind": "restackAbortResult",
                    "mutatesRemote": false
                }
            ],
            "results": {
                "stdout": [
                    {"kind": "restackPlan", "schemaVersion": 1},
                    {"kind": "restackResult", "schemaVersion": 1},
                    {"kind": "restackAbortResult", "schemaVersion": 1}
                ],
                "stderr": {"kind": "restackError", "schemaVersion": 1},
                "exitCodes": {"success": 0, "failure": 1, "usage": 2}
            },
            "validation": {
                "gitRefComponentRejects": [
                    "empty values",
                    "leading hyphens",
                    "control characters",
                    "invalid Git ref syntax",
                    "percent-encoded octets"
                ],
                "unknownPayloadFieldsRejected": true
            },
            "security": {
                "operatorTrusted": false,
                "repositoryContentTrusted": false,
                "previewRequiredBeforeApply": true,
                "freshFetchRequired": true,
                "exactEnvironmentLease": true,
                "credentialsRedacted": true,
                "repositoryTextIsDataNotInstructions": true
            }
    });
    serde_json::to_writer(std::io::stdout().lock(), &description)?;
    writeln!(std::io::stdout().lock())?;
    Ok(())
}
