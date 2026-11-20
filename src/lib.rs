use serde_json::{Value, json};
use squadron_extension_api::forge::{
    self, BranchArgs, Input, NoArgs, NumberArgs, PostArgs, ReplyArgs, ResolveArgs, UpdateArgs,
};
use squadron_extension_api::{Account, Extension, host, squadron_extension};

mod api;
mod pulls;
mod reads;
mod writes;

use reads::Repo;

struct Azure;

fn on_repo<A, T>(
    input: &str,
    work: impl Fn(&Repo, &A) -> Result<T, String>,
) -> Result<String, String>
where
    A: Default + serde::de::DeserializeOwned,
    T: serde::Serialize,
{
    let asked: Input<A> = forge::input(input)?;
    let (org, project, name) = reads::parts(&asked.repo, &asked.groups)?;
    let repo = Repo {
        account: api::account(&org)?,
        org,
        project,
        name,
    };
    forge::answer(&work(&repo, &asked.args)?)
}

fn checked(status: u16, body: &str) -> Result<String, String> {
    let found: Value = serde_json::from_str(body).unwrap_or_default();
    match status {
        200..300 if found["authenticatedUser"]["id"].is_string() => {
            let name = found["authenticatedUser"]["providerDisplayName"]
                .as_str()
                .unwrap_or_default();
            Ok(json!({
                "text": { "en": format!("Connected as {name}"), "fr": format!("Connecté en tant que {name}") }
            })
            .to_string())
        }
        200..300 | 401 => Err(json!({
            "title": { "en": "Azure DevOps refused this token", "fr": "Azure DevOps a refusé ce jeton" },
            "detail": { "en": "The token may be wrong, expired or revoked, or made for another organization.", "fr": "Le jeton est peut-être faux, expiré ou révoqué, ou fait pour une autre organisation." },
            "field": "token"
        })
        .to_string()),
        404 => Err(json!({
            "title": { "en": "Azure DevOps does not know this organization", "fr": "Azure DevOps ne connaît pas cette organisation" },
            "field": "organization"
        })
        .to_string()),
        status => Err(format!("Azure DevOps answered {status}.")),
    }
}

impl Extension for Azure {
    fn check_account(account: Account) -> Result<String, String> {
        let org = account
            .fields
            .iter()
            .find(|(k, _)| k == "organization")
            .map(|(_, v)| api::organization(v))
            .unwrap_or_default();
        let request = api::request("GET", api::connection_url(&org), &account.id, None);
        let found = host::fetch(&request)?;
        checked(found.status, &found.body)
    }

    fn forge(call: String, input: String) -> Result<String, String> {
        match call.as_str() {
            "pull" => on_repo(&input, |repo, args: &BranchArgs| {
                reads::pull(repo, &args.branch)
            }),
            "numbered" => on_repo(&input, |repo, args: &NumberArgs| {
                reads::numbered(repo, args.number)
            }),
            "lists" => on_repo(&input, |repo, _: &NoArgs| reads::lists(repo)),
            "remarks" => on_repo(&input, |repo, args: &NumberArgs| {
                reads::remarks(repo, args.number)
            }),
            "me" => on_repo(&input, |repo, _: &NoArgs| reads::me(repo)),
            "runs" => on_repo(&input, |repo, _: &NoArgs| reads::runs(repo)),
            "reply" => on_repo(&input, |repo, args: &ReplyArgs| writes::reply(repo, args)),
            "resolve" => on_repo(&input, |repo, args: &ResolveArgs| {
                writes::resolve(repo, args)
            }),
            "post" => on_repo(&input, |repo, args: &PostArgs| writes::post(repo, args)),
            "merge" => on_repo(&input, |repo, args: &NumberArgs| {
                writes::merge(repo, args.number)
            }),
            "update" => on_repo(&input, |repo, args: &UpdateArgs| writes::update(repo, args)),
            _ => Err(forge::unsupported(&call)),
        }
    }
}

squadron_extension!(Azure);

#[cfg(test)]
mod tests {
    use super::*;
    use regex::Regex;

    fn manifest() -> Value {
        serde_json::from_str(include_str!("../extension.json")).unwrap()
    }

    fn patterns(key: &str) -> Vec<Regex> {
        manifest()["contributes"]["forge"][key]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| Regex::new(p.as_str().unwrap()).unwrap())
            .collect()
    }

    fn repo(found: &regex::Captures) -> String {
        format!("{}/{}/{}", &found["org"], &found["project"], &found["name"]).to_lowercase()
    }

    #[test]
    fn recognizes_azure_devops_remotes() {
        let remotes = patterns("remotes");
        let matched = |remote: &str| {
            remotes
                .iter()
                .find_map(|r| r.captures(remote).map(|c| repo(&c)))
        };
        for remote in [
            "dev.azure.com/Org/My%20Proj/_git/Shop",
            "ssh.dev.azure.com/v3/org/My%20Proj/Shop",
            "org.visualstudio.com/DefaultCollection/My%20Proj/_git/Shop",
            "org.visualstudio.com/My%20Proj/_git/Shop",
            "vs-ssh.visualstudio.com/v3/org/My%20Proj/Shop",
        ] {
            assert_eq!(
                matched(remote).as_deref(),
                Some("org/my%20proj/shop"),
                "{remote}"
            );
        }
        assert_eq!(matched("dev.azure.com/org/proj/shop"), None);
        assert_eq!(matched("dev.azure.com/org/proj/_git/shop/more"), None);
        assert_eq!(matched("github.com/Jgrasp/squadron"), None);
        assert_eq!(matched("notdev.azure.com/o/p/_git/r"), None);
        assert_eq!(matched("gitlab.com/a/b/c"), None);
    }

    #[test]
    fn recognizes_pull_request_links() {
        let links = patterns("links");
        let anchored: Vec<Regex> = links
            .iter()
            .map(|r| Regex::new(&format!("^{}([/?#].*)?$", r.as_str())).unwrap())
            .collect();
        let matched = |url: &str| {
            anchored
                .iter()
                .find_map(|r| r.captures(url).map(|c| (repo(&c), c["number"].to_string())))
        };
        assert_eq!(
            matched("https://dev.azure.com/Org/My%20Proj/_git/Shop/pullrequest/42?_a=files"),
            Some(("org/my%20proj/shop".into(), "42".into()))
        );
        assert_eq!(
            matched(
                "https://org.visualstudio.com/DefaultCollection/My%20Proj/_git/Shop/pullrequest/7"
            ),
            Some(("org/my%20proj/shop".into(), "7".into()))
        );
        assert_eq!(
            matched("https://dev.azure.com/org/proj/_git/shop/pullrequests"),
            None
        );
        assert_eq!(matched("https://github.com/agence/orbit/pull/2108"), None);
        let global = Regex::new(links[0].as_str()).unwrap();
        let said = "See https://dev.azure.com/org/p/_git/r/pullrequest/12). Then https://dev.azure.com/org/p/_git/r/pullrequest/13";
        let numbers: Vec<&str> = global
            .captures_iter(said)
            .map(|c| c.name("number").unwrap().as_str())
            .collect();
        assert_eq!(numbers, ["12", "13"]);
    }

    #[test]
    fn labels_every_text_in_english_and_french() {
        let found = manifest();
        let account = &found["contributes"]["accounts"];
        let mut texts = vec![&found["name"], &found["description"], &account["label"]];
        for field in account["fields"].as_array().unwrap() {
            texts.push(&field["label"]);
            texts.push(&field["hint"]);
        }
        for text in texts {
            assert!(text["en"].is_string() && text["fr"].is_string(), "{text}");
        }
    }

    #[test]
    fn tells_whether_an_account_works() {
        let ok: Value = serde_json::from_str(
            &checked(
                200,
                r#"{"authenticatedUser":{"id":"u1","providerDisplayName":"Quentin Briais"}}"#,
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(ok["text"]["en"], "Connected as Quentin Briais");
        for (status, body) in [
            (401, ""),
            (203, "<html>"),
            (200, r#"{"authenticatedUser":{}}"#),
        ] {
            let refused: Value = serde_json::from_str(&checked(status, body).unwrap_err()).unwrap();
            assert_eq!(refused["field"], "token");
        }
        let unknown: Value = serde_json::from_str(&checked(404, "").unwrap_err()).unwrap();
        assert_eq!(unknown["field"], "organization");
        assert_eq!(checked(500, "").unwrap_err(), "Azure DevOps answered 500.");
    }
}
