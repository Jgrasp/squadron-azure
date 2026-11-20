# Azure DevOps

A [Squadron](https://github.com/Jgrasp/squadron) extension that brings the pull requests and pipeline builds of your Azure DevOps projects into Squadron.

## Use

In Settings › Extensions › Azure DevOps, add an account with the name of your organization (or its address, `https://dev.azure.com/<organization>`) and a [personal access token](https://learn.microsoft.com/azure/devops/organizations/accounts/use-personal-access-tokens-to-authenticate): in Azure DevOps, open User settings › Personal access tokens, choose **New Token**, name it Squadron, pick **Custom defined**, then tick these scopes:

- **Code**: Read & write
- **Build**: Read

**Test the connection** checks the token before you save it. Add one account per organization: each repository uses the account of its organization, or the first one when none matches.

Every project whose `origin` is on `dev.azure.com`, `ssh.dev.azure.com` or `<organization>.visualstudio.com` then gets, in Squadron:

- the pull request of each agent's branch, and chips for the pull request links in conversations;
- the pull requests to review, yours and the others, with their checks and reviewers;
- reviews with line comments, a summary and a vote, replies to remarks, resolved threads, merges, and title and description changes, from you or from an agent;
- the pipeline builds, with a notification when one ends.

## What it asks Squadron for

- HTTPS requests to `dev.azure.com`, Azure DevOps' API. Repositories cloned from `*.visualstudio.com` are read through `dev.azure.com` too.
- Azure DevOps accounts: each token stays in the system keyring. Squadron adds the organization and token to the requests to Azure DevOps, and the extension never reads the token.
  - **Code (Read & write)** reads the pull requests, their threads and statuses, and comments, votes, resolves threads, merges and edits them.
  - **Build (Read)** shows the pipeline builds.

## Build

```sh
rustup target add wasm32-wasip2
cargo build --release --target wasm32-wasip2
cargo test
```

CI checks the formatting of every push and pull request with `cargo fmt --check`.
