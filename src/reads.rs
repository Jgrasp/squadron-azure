use crate::api::{self, SITE};
use crate::pulls::{self, branch, list, spoken, text};
use serde_json::Value;
use squadron_extension_api::forge::{Lists, Numbered, Person, Pull, Remark, Run};
use squadron_extension_api::host;
use std::collections::BTreeMap;

pub struct Repo {
    pub account: String,
    pub org: String,
    pub project: String,
    pub name: String,
}

pub fn parts(
    repo: &str,
    groups: &BTreeMap<String, String>,
) -> Result<(String, String, String), String> {
    let found = match (groups.get("org"), groups.get("project"), groups.get("name")) {
        (Some(org), Some(project), Some(name)) => {
            Some((org.as_str(), project.as_str(), name.as_str()))
        }
        _ => {
            let mut parts = repo.splitn(3, '/');
            match (parts.next(), parts.next(), parts.next()) {
                (Some(org), Some(project), Some(name)) => Some((org, project, name)),
                _ => None,
            }
        }
    };
    found
        .filter(|(o, p, n)| ![o, p, n].iter().any(|s| s.is_empty()) && !n.contains('/'))
        .map(|(o, p, n)| (o.to_lowercase(), p.to_lowercase(), n.to_lowercase()))
        .ok_or_else(|| format!("{repo} is not an Azure DevOps repository."))
}

impl Repo {
    pub fn slug(&self) -> String {
        format!("{}/{}/{}", self.org, self.project, self.name)
    }

    pub fn repository(&self) -> String {
        format!(
            "{SITE}/{}/{}/_apis/git/repositories/{}",
            self.org, self.project, self.name
        )
    }

    pub fn pull_api(&self, number: u64) -> String {
        format!("{}/pullRequests/{number}", self.repository())
    }

    pub fn get(&self, url: &str, query: &[(&str, &str)]) -> Result<Value, String> {
        api::send(&self.account, "GET", url, query, None)
    }
}

pub fn me_id(repo: &Repo) -> Result<String, String> {
    let key = format!("me:{}:{}", repo.account, repo.org);
    if let Some(found) = host::state_get(&key) {
        return Ok(found);
    }
    let id = text(&api::connection(&repo.account, &repo.org)?["id"]);
    host::state_set(&key, Some(&id));
    Ok(id)
}

pub fn filled(repo: &Repo, found: &[&Value], me: &str) -> Vec<Pull> {
    let slug = repo.slug();
    let urls: Vec<String> = found
        .iter()
        .flat_map(|pull| {
            let path = repo.pull_api(pull["pullRequestId"].as_u64().unwrap_or_default());
            [format!("{path}/statuses"), format!("{path}/threads")]
        })
        .collect();
    let mut answers = api::many(&repo.account, &urls).into_iter();
    found
        .iter()
        .map(|pull| {
            let statuses = answers.next().unwrap_or_default();
            let threads = answers.next().unwrap_or_default();
            pulls::pull_of(pull, &statuses, &threads, me, &slug)
        })
        .collect()
}

fn search(repo: &Repo, query: &[(&str, &str)]) -> Result<Vec<Value>, String> {
    let found = repo.get(&format!("{}/pullrequests", repo.repository()), query)?;
    Ok(list(&found["value"]).to_vec())
}

pub fn pull(repo: &Repo, branch: &str) -> Result<Option<Pull>, String> {
    let source = format!("refs/heads/{branch}");
    let found = search(
        repo,
        &[
            ("searchCriteria.sourceRefName", &source),
            ("searchCriteria.status", "all"),
            ("$top", "1"),
        ],
    )?;
    let me = me_id(repo).unwrap_or_default();
    Ok(filled(repo, &found.iter().take(1).collect::<Vec<_>>(), &me)
        .into_iter()
        .next())
}

pub fn numbered(repo: &Repo, number: u64) -> Result<Numbered, String> {
    let found = repo.get(&repo.pull_api(number), &[])?;
    let me = me_id(repo).unwrap_or_default();
    Ok(Numbered {
        pull: filled(repo, &[&found], &me).remove(0),
        ..Numbered::default()
    })
}

pub fn sorted(found: [Vec<Value>; 3]) -> [Vec<Value>; 3] {
    let [review, mine, open] = found;
    let number = |p: &Value| p["pullRequestId"].as_u64();
    let others = open
        .into_iter()
        .filter(|p| !review.iter().chain(&mine).any(|f| number(f) == number(p)))
        .collect();
    [review, mine, others]
}

pub fn lists(repo: &Repo) -> Result<Lists, String> {
    let me = me_id(repo)?;
    let mut found: [Vec<Value>; 3] = Default::default();
    for (i, who) in [
        Some("searchCriteria.reviewerId"),
        Some("searchCriteria.creatorId"),
        None,
    ]
    .into_iter()
    .enumerate()
    {
        let mut query = vec![("searchCriteria.status", "active"), ("$top", "50")];
        if let Some(who) = who {
            query.push((who, &me));
        }
        found[i] = search(repo, &query)?;
    }
    let sorted = sorted(found);
    let all: Vec<&Value> = sorted.iter().flatten().collect();
    let mut pulls = filled(repo, &all, &me).into_iter();
    let [review, mine, others] = sorted.map(|l| pulls.by_ref().take(l.len()).collect());
    Ok(Lists {
        review,
        mine,
        others,
    })
}

pub fn remarks_of(threads: &Value) -> Vec<Remark> {
    spoken(threads)
        .filter_map(|(thread, comment)| {
            let tid = thread["id"].as_u64()?;
            let id = comment["id"].as_u64()?;
            let context = &thread["threadContext"];
            let path = context["filePath"]
                .as_str()
                .map(|p| p.trim_start_matches('/').to_string());
            let right = context["rightFileStart"]["line"].as_u64();
            let left = context["leftFileStart"]["line"].as_u64();
            let parent = comment["parentCommentId"].as_u64().unwrap_or_default();
            let status = thread["status"].as_str().filter(|s| *s != "unknown");
            Some(Remark {
                id: format!("{tid}:{id}"),
                author: text(&comment["author"]["displayName"]),
                avatar: String::new(),
                body: text(&comment["content"]),
                kind: if path.is_some() { "line" } else { "general" }.into(),
                path,
                line: right.or(left).map(|n| n as u32),
                removed: right.is_none() && left.is_some(),
                reply_to: (parent > 0).then(|| format!("{tid}:{parent}")),
                at: text(&comment["publishedDate"]),
                resolved: status
                    .filter(|_| parent == 0)
                    .map(|s| !matches!(s, "active" | "pending")),
                commit: None,
            })
        })
        .collect()
}

pub fn remarks(repo: &Repo, number: u64) -> Result<Vec<Remark>, String> {
    let threads = repo.get(&format!("{}/threads", repo.pull_api(number)), &[])?;
    let mut found = remarks_of(&threads);
    found.sort_by(|a, b| a.at.cmp(&b.at));
    Ok(found)
}

pub fn me(repo: &Repo) -> Result<Person, String> {
    let user = api::connection(&repo.account, &repo.org)?;
    Ok(Person {
        name: text(&user["providerDisplayName"]),
        avatar: String::new(),
    })
}

fn pull_in(reference: &str) -> Option<u64> {
    reference
        .strip_prefix("refs/pull/")?
        .split('/')
        .next()?
        .parse()
        .ok()
}

pub fn run_of(value: &Value) -> Run {
    let status = match (
        value["status"].as_str().unwrap_or_default(),
        value["result"].as_str().unwrap_or_default(),
    ) {
        ("completed", "succeeded") => "success",
        ("completed", "canceled") => "cancelled",
        ("completed", _) => "failure",
        ("inProgress" | "cancelling", _) => "running",
        _ => "queued",
    };
    let id = value["id"].as_u64().unwrap_or_default();
    Run {
        id: id.to_string(),
        number: id,
        name: text(&value["definition"]["name"]),
        title: text(&value["triggerInfo"]["ci.message"])
            .lines()
            .next()
            .unwrap_or_default()
            .to_string(),
        event: text(&value["reason"]),
        branch: branch(&value["sourceBranch"]),
        status: status.into(),
        url: text(&value["_links"]["web"]["href"]),
        started: value["startTime"]
            .as_str()
            .or(value["queueTime"].as_str())
            .unwrap_or_default()
            .into(),
        finished: (value["status"] == "completed").then(|| text(&value["finishTime"])),
        author: text(&value["requestedFor"]["displayName"]),
        author_avatar: String::new(),
        pull: pull_in(&text(&value["sourceBranch"])),
    }
}

pub fn runs(repo: &Repo) -> Result<Vec<Run>, String> {
    let id = text(&repo.get(&repo.repository(), &[])?["id"]);
    let found = repo.get(
        &format!("{SITE}/{}/{}/_apis/build/builds", repo.org, repo.project),
        &[
            ("repositoryId", &id),
            ("repositoryType", "TfsGit"),
            ("$top", "20"),
            ("queryOrder", "queueTimeDescending"),
        ],
    )?;
    Ok(list(&found["value"]).iter().map(run_of).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pulls::tests::threads;
    use serde_json::json;

    #[test]
    fn reads_remarks_as_thread_and_comment() {
        let remarks = remarks_of(&threads());
        let seen: Vec<_> = remarks
            .iter()
            .map(|r| {
                (
                    r.id.as_str(),
                    r.path.as_deref(),
                    r.line,
                    r.reply_to.as_deref(),
                    r.kind.as_str(),
                )
            })
            .collect();
        assert_eq!(
            seen,
            [
                ("11:1", Some("src/a.cs"), Some(12), None, "line"),
                ("11:2", Some("src/a.cs"), Some(12), Some("11:1"), "line")
            ]
        );
        assert_eq!(
            (remarks[0].resolved, remarks[1].resolved),
            (Some(true), None)
        );
        assert_eq!(
            (remarks[0].author.as_str(), remarks[0].body.as_str()),
            ("Ann", "Rename")
        );
        let more = json!({ "value": [
            { "id": 20, "status": "active", "threadContext": { "filePath": "/src/b.cs", "leftFileStart": { "line": 4 } }, "comments": [
                { "id": 1, "parentCommentId": 0, "content": "Why?", "commentType": "text", "publishedDate": "t3" }
            ] },
            { "id": 21, "status": "unknown", "comments": [
                { "id": 1, "parentCommentId": 0, "content": "LGTM", "commentType": "text", "publishedDate": "t4" }
            ] },
            { "id": 22, "status": "pending", "comments": [
                { "id": 1, "parentCommentId": 0, "content": "Hm", "commentType": "text" }
            ] }
        ] });
        let found = remarks_of(&more);
        let seen: Vec<_> = found
            .iter()
            .map(|r| (r.line, r.removed, r.kind.as_str(), r.resolved))
            .collect();
        assert_eq!(
            seen,
            [
                (Some(4), true, "line", Some(false)),
                (None, false, "general", None),
                (None, false, "general", Some(false))
            ]
        );
    }

    #[test]
    fn sums_up_a_build() {
        let run = run_of(
            &json!({ "id": 1663614, "buildNumber": "20261104.17", "status": "completed", "result": "partiallySucceeded", "queueTime": "q", "startTime": "s", "finishTime": "f", "sourceBranch": "refs/heads/main", "reason": "individualCI", "definition": { "name": "maui-pr" }, "requestedFor": { "displayName": "Microsoft.VisualStudio.Services.TFS" }, "triggerInfo": { "ci.message": "[iOS] Fix badge\n\nmore" }, "_links": { "web": { "href": "https://dev.azure.com/o/p/_build/results?buildId=1663614" } } }),
        );
        assert_eq!(
            (
                run.id.as_str(),
                run.number,
                run.name.as_str(),
                run.title.as_str(),
                run.branch.as_str(),
                run.status.as_str(),
                run.started.as_str(),
                run.finished.as_deref()
            ),
            (
                "1663614",
                1663614,
                "maui-pr",
                "[iOS] Fix badge",
                "main",
                "failure",
                "s",
                Some("f")
            )
        );
        assert_eq!(
            (
                run.event.as_str(),
                run.author.as_str(),
                run.url.as_str(),
                run.pull
            ),
            (
                "individualCI",
                "Microsoft.VisualStudio.Services.TFS",
                "https://dev.azure.com/o/p/_build/results?buildId=1663614",
                None
            )
        );
        assert_eq!(
            run_of(&json!({ "sourceBranch": "refs/pull/482/merge", "reason": "pullRequest" })).pull,
            Some(482)
        );
        let queued = run_of(&json!({ "status": "notStarted", "queueTime": "q" }));
        assert_eq!(
            (
                queued.status.as_str(),
                queued.started.as_str(),
                queued.finished
            ),
            ("queued", "q", None)
        );
        let status = |status: &str, result: &str| {
            run_of(&json!({ "status": status, "result": result })).status
        };
        assert_eq!(status("completed", "succeeded"), "success");
        assert_eq!(status("completed", "canceled"), "cancelled");
        assert_eq!(status("inProgress", ""), "running");
        assert_eq!(status("cancelling", ""), "running");
    }

    #[test]
    fn sorts_pull_requests_into_the_three_lists() {
        let pr = |id: u64| json!({ "pullRequestId": id });
        let [review, mine, others] =
            sorted([vec![pr(1), pr(2)], vec![pr(3)], (1..=5).map(pr).collect()]);
        let numbers = |list: &[Value]| {
            list.iter()
                .map(|p| p["pullRequestId"].as_u64().unwrap())
                .collect::<Vec<_>>()
        };
        assert_eq!(numbers(&review), [1, 2]);
        assert_eq!(numbers(&mine), [3]);
        assert_eq!(numbers(&others), [4, 5]);
    }

    #[test]
    fn reads_the_organization_project_and_name_of_a_repo() {
        let groups = BTreeMap::from([
            ("org".to_string(), "Org".to_string()),
            ("project".to_string(), "My%20Proj".to_string()),
            ("name".to_string(), "Shop".to_string()),
        ]);
        let lowered = (
            "org".to_string(),
            "my%20proj".to_string(),
            "shop".to_string(),
        );
        assert_eq!(parts("x", &groups).unwrap(), lowered);
        assert_eq!(
            parts("Org/My%20Proj/Shop", &BTreeMap::new()).unwrap(),
            lowered
        );
        assert!(parts("a/b", &BTreeMap::new()).is_err());
        assert!(parts("a//c", &BTreeMap::new()).is_err());
    }
}
