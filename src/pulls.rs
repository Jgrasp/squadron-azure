use crate::api::SITE;
use serde_json::Value;
use squadron_extension_api::forge::{Pull, Reviewer};

pub fn text(value: &Value) -> String {
    value.as_str().unwrap_or_default().to_string()
}

pub fn list(value: &Value) -> &[Value] {
    value.as_array().map(Vec::as_slice).unwrap_or_default()
}

pub fn branch(value: &Value) -> String {
    text(value).trim_start_matches("refs/heads/").to_string()
}

fn verdict(person: &Value) -> &'static str {
    match person["vote"].as_i64().unwrap_or_default() {
        v if v > 0 => "approved",
        v if v < 0 => "changes_requested",
        _ => "waiting",
    }
}

pub fn spoken(threads: &Value) -> impl Iterator<Item = (&Value, &Value)> {
    list(&threads["value"])
        .iter()
        .filter(|t| t["isDeleted"] != true)
        .flat_map(|t| {
            list(&t["comments"])
                .iter()
                .filter(|c| c["commentType"] == "text" && c["isDeleted"] != true)
                .map(move |c| (t, c))
        })
}

fn checks(statuses: &Value) -> Option<String> {
    let states: Vec<&str> = list(&statuses["value"])
        .iter()
        .filter_map(|s| match s["state"].as_str() {
            Some("succeeded") => Some("pass"),
            Some("pending") => Some("pending"),
            Some("failed" | "error") => Some("fail"),
            _ => None,
        })
        .collect();
    if states.is_empty() {
        None
    } else if states.contains(&"fail") {
        Some("failing".into())
    } else if states.contains(&"pending") {
        Some("pending".into())
    } else {
        Some("passing".into())
    }
}

pub const MERGE_STRATEGY: &str = "fa4e907d-c16b-4a4c-9dfa-4916e5d171ab";

fn covers(scope: &Value, repository: &Value, refname: &str) -> bool {
    let id = &scope["repositoryId"];
    let target = scope["refName"].as_str();
    (id.is_null() || *id == repository["id"])
        && match scope["matchKind"]
            .as_str()
            .map(str::to_lowercase)
            .as_deref()
        {
            Some("exact") => target == Some(refname),
            Some("prefix") => target.is_some_and(|t| refname.starts_with(t)),
            Some("defaultbranch") => repository["defaultBranch"] == refname,
            _ => target.is_none(),
        }
}

fn applying<'a>(
    policies: &'a [Value],
    repository: &'a Value,
    refname: &'a str,
) -> impl Iterator<Item = &'a Value> {
    policies.iter().filter(move |p| {
        p["isEnabled"] == true
            && p["isBlocking"] == true
            && p["isDeleted"] != true
            && list(&p["settings"]["scope"])
                .iter()
                .any(|s| covers(s, repository, refname))
    })
}

fn allowed(settings: &Value) -> [bool; 3] {
    let keys = ["allowNoFastForward", "allowSquash", "allowRebase"];
    if keys.iter().any(|k| settings[k].is_boolean()) {
        keys.map(|k| settings[k] == true)
    } else {
        let squash = settings["useSquashMerge"] == true;
        [!squash, squash, false]
    }
}

pub fn merging(pull: &mut Pull, found: &Value, policies: &Value, repository: &Value) {
    let Some(policies) = policies.get("value").unwrap_or(policies).as_array() else {
        return;
    };
    let source = text(&found["sourceRefName"]);
    let target = text(&found["targetRefName"]);
    let protected = applying(policies, repository, &source).next().is_some();
    let mut open = [true, true, !protected];
    for policy in applying(policies, repository, &target).filter(|p| {
        p["type"]["id"] == MERGE_STRATEGY || p["type"]["displayName"] == "Require a merge strategy"
    }) {
        let rule = allowed(&policy["settings"]);
        open = [0, 1, 2].map(|i| open[i] && rule[i]);
    }
    pull.merge_methods = ["merge", "squash", "rebase"]
        .into_iter()
        .zip(open)
        .filter(|(_, o)| *o)
        .map(|(m, _)| m.to_string())
        .collect();
    pull.branch_protected = protected;
    pull.delete_branch =
        Some(!protected && found["completionOptions"]["deleteSourceBranch"] == true);
    pull.delete_branch_settable = true;
}

pub fn pull_of(found: &Value, statuses: &Value, threads: &Value, me: &str, slug: &str) -> Pull {
    let people = list(&found["reviewers"]);
    let approvals = people.iter().filter(|p| verdict(p) == "approved").count() as u32;
    let changes_requested = people.iter().any(|p| verdict(p) == "changes_requested");
    let checks = checks(statuses);
    let state = match found["status"].as_str().unwrap_or_default() {
        "active" => "open",
        "completed" => "merged",
        "abandoned" => "closed",
        other => other,
    };
    let draft = found["isDraft"] == true;
    let number = found["pullRequestId"].as_u64().unwrap_or_default();
    let mut parts = slug.splitn(3, '/');
    let (org, project, name) = (
        parts.next().unwrap_or_default(),
        parts.next().unwrap_or_default(),
        parts.next().unwrap_or_default(),
    );
    Pull {
        number,
        title: text(&found["title"]),
        description: text(&found["description"]),
        url: format!("{SITE}/{org}/{project}/_git/{name}/pullrequest/{number}"),
        state: state.into(),
        draft,
        approvals,
        changes_requested,
        comments: spoken(threads).count() as u32,
        mergeable: state == "open"
            && !draft
            && found["mergeStatus"] == "succeeded"
            && approvals > 0
            && !changes_requested
            && checks.as_deref().is_none_or(|c| c == "passing"),
        checks,
        branch: branch(&found["sourceRefName"]),
        base: branch(&found["targetRefName"]),
        head: text(&found["lastMergeSourceCommit"]["commitId"]),
        author: text(&found["createdBy"]["displayName"]),
        author_avatar: String::new(),
        updated: found["closedDate"]
            .as_str()
            .or(found["creationDate"].as_str())
            .unwrap_or_default()
            .into(),
        reviewed: people
            .iter()
            .find(|p| !me.is_empty() && p["id"] == me)
            .map(verdict)
            .filter(|v| *v != "waiting")
            .map(str::to_string),
        reviewed_head: None,
        reviewed_at: None,
        head_at: None,
        reviewers: people
            .iter()
            .map(|p| Reviewer {
                name: text(&p["displayName"]),
                avatar: String::new(),
                verdict: verdict(p).into(),
            })
            .collect(),
        conflicts: found["mergeStatus"] == "conflicts",
        ..Pull::default()
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use serde_json::json;

    pub fn threads() -> Value {
        json!({ "value": [
            { "id": 8, "comments": [{ "id": 1, "parentCommentId": 0, "content": "MerlinBot added 3 reviewers", "commentType": "system" }] },
            { "id": 11, "status": "fixed", "threadContext": { "filePath": "/src/a.cs", "rightFileStart": { "line": 12, "offset": 1 } }, "comments": [
                { "id": 1, "parentCommentId": 0, "content": "Rename", "commentType": "text", "author": { "displayName": "Ann" }, "publishedDate": "t1" },
                { "id": 2, "parentCommentId": 1, "content": "Done", "commentType": "text", "author": { "displayName": "Bob" }, "publishedDate": "t2" },
                { "id": 3, "parentCommentId": 1, "commentType": "text", "isDeleted": true }
            ] },
            { "id": 12, "isDeleted": true, "comments": [{ "id": 1, "content": "x", "commentType": "text" }] }
        ] })
    }

    #[test]
    fn sums_up_an_azure_devops_pull_request() {
        let found = json!({
            "pullRequestId": 5, "status": "active", "title": "Inventory bootstrapping", "description": "Adds metadata",
            "createdBy": { "displayName": "MerlinBot", "id": "56b0" }, "creationDate": "2026-02-18T04:35:01Z",
            "sourceRefName": "refs/heads/invBootstrap", "targetRefName": "refs/heads/main", "mergeStatus": "succeeded", "isDraft": false,
            "lastMergeSourceCommit": { "commitId": "3aae318f" },
            "reviewers": [{ "vote": 10, "displayName": "Mark Wilkie", "id": "08c3" }, { "vote": 0, "displayName": "[public]\\Reviewers", "id": "ab54", "isContainer": true }]
        });
        let statuses = json!({ "value": [{ "state": "succeeded" }, { "state": "notApplicable" }] });
        assert_eq!(
            pull_of(&found, &statuses, &threads(), "08c3", "org/my%20proj/shop"),
            Pull {
                number: 5,
                title: "Inventory bootstrapping".into(),
                description: "Adds metadata".into(),
                url: "https://dev.azure.com/org/my%20proj/_git/shop/pullrequest/5".into(),
                state: "open".into(),
                draft: false,
                approvals: 1,
                changes_requested: false,
                comments: 2,
                checks: Some("passing".into()),
                branch: "invBootstrap".into(),
                base: "main".into(),
                head: "3aae318f".into(),
                author: "MerlinBot".into(),
                author_avatar: String::new(),
                updated: "2026-02-18T04:35:01Z".into(),
                reviewed: Some("approved".into()),
                reviewed_head: None,
                reviewed_at: None,
                head_at: None,
                reviewers: vec![
                    Reviewer {
                        name: "Mark Wilkie".into(),
                        avatar: String::new(),
                        verdict: "approved".into()
                    },
                    Reviewer {
                        name: "[public]\\Reviewers".into(),
                        avatar: String::new(),
                        verdict: "waiting".into()
                    },
                ],
                conflicts: false,
                mergeable: true,
                ..Pull::default()
            }
        );
        let mut rejected = found.clone();
        rejected["reviewers"][1]["vote"] = json!(-5);
        rejected["status"] = json!("completed");
        rejected["closedDate"] = json!("2026-02-19T00:00:00Z");
        let pull = pull_of(&rejected, &Value::Null, &Value::Null, "", "o/p/r");
        assert_eq!(
            (
                pull.state.as_str(),
                pull.changes_requested,
                pull.mergeable,
                pull.updated.as_str(),
                pull.reviewed
            ),
            ("merged", true, false, "2026-02-19T00:00:00Z", None)
        );
        let mut conflicted = found.clone();
        conflicted["mergeStatus"] = json!("conflicts");
        let pull = pull_of(&conflicted, &statuses, &Value::Null, "", "o/p/r");
        assert_eq!((pull.conflicts, pull.mergeable), (true, false));
        let failed = json!({ "value": [{ "state": "succeeded" }, { "state": "error" }, { "state": "pending" }] });
        assert_eq!(
            pull_of(&found, &failed, &Value::Null, "", "o/p/r")
                .checks
                .as_deref(),
            Some("failing")
        );
        let pending = json!({ "value": [{ "state": "succeeded" }, { "state": "pending" }] });
        let pull = pull_of(&found, &pending, &Value::Null, "", "o/p/r");
        assert_eq!(
            (pull.checks.as_deref(), pull.mergeable),
            (Some("pending"), false)
        );
        let mut draft = found.clone();
        draft["isDraft"] = json!(true);
        draft["status"] = json!("abandoned");
        let pull = pull_of(&draft, &Value::Null, &Value::Null, "", "o/p/r");
        assert_eq!((pull.state.as_str(), pull.draft), ("closed", true));
    }

    #[test]
    fn reads_merge_types_and_protection_from_branch_policies() {
        let repository = json!({ "id": "r1", "defaultBranch": "refs/heads/main" });
        let found = json!({
            "sourceRefName": "refs/heads/feat/x", "targetRefName": "refs/heads/main",
            "completionOptions": { "deleteSourceBranch": true }
        });
        let policy = |scope: Value, settings: Value| {
            let mut settings = settings;
            settings["scope"] = json!([scope]);
            json!({ "isEnabled": true, "isBlocking": true, "type": { "id": MERGE_STRATEGY }, "settings": settings })
        };
        let read = |policies: Value| {
            let mut pull = Pull::default();
            merging(&mut pull, &found, &policies, &repository);
            (
                pull.merge_methods,
                pull.delete_branch,
                pull.branch_protected,
            )
        };
        let all = vec!["merge".to_string(), "squash".into(), "rebase".into()];
        assert_eq!(
            read(json!({ "count": 0, "value": [] })),
            (all.clone(), Some(true), false)
        );
        assert_eq!(read(Value::Null), (vec![], None, false));
        let exact = policy(
            json!({ "repositoryId": "r1", "refName": "refs/heads/main", "matchKind": "Exact" }),
            json!({ "allowSquash": true, "allowRebase": true, "allowNoFastForward": false }),
        );
        let wide = policy(
            json!({ "repositoryId": null, "matchKind": "DefaultBranch" }),
            json!({ "allowSquash": true, "allowNoFastForward": true }),
        );
        assert_eq!(read(json!({ "value": [exact, wide] })).0, vec!["squash"]);
        let mut disabled = exact.clone();
        disabled["isEnabled"] = json!(false);
        let other = policy(
            json!({ "repositoryId": "r2", "refName": "refs/heads/main", "matchKind": "Exact" }),
            json!({ "allowSquash": true }),
        );
        assert_eq!(read(json!([disabled, other])).0, all);
        let legacy = policy(
            json!({ "repositoryId": "r1", "refName": "refs/heads/", "matchKind": "prefix" }),
            json!({ "useSquashMerge": true }),
        );
        assert_eq!(
            read(json!([legacy])),
            (vec!["squash".to_string()], Some(false), true)
        );
        let reviewers = json!({
            "isEnabled": true, "isBlocking": true, "type": { "id": "other" },
            "settings": { "scope": [{ "repositoryId": "r1", "refName": "refs/heads/feat/", "matchKind": "Prefix" }] }
        });
        assert_eq!(
            read(json!([reviewers])),
            (
                vec!["merge".to_string(), "squash".into()],
                Some(false),
                true
            )
        );
    }
}
