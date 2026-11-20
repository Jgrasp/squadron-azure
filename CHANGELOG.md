# Changelog

## 0.3.0 · 2026-11-20

- Open a pull request from an agent's pushed branch, as a draft if you like, with Squadron's Open a pull request button
- A description longer than the 4000 characters Azure DevOps takes is cut at 4000
- A branch that already has an active pull request into the same base gets a clear message

## 0.2.0 · 2026-11-20

- Merging a pull request offers the merge types its target branch allows: merge, squash, or rebase and fast-forward
- Merging can delete the source branch, as the pull request's completion options say, unless branch policies protect it

## 0.1.0 · 2026-11-20

- Pull requests of your Azure DevOps projects in Squadron: the pull request of each agent's branch, the ones to review, yours and the others, with their checks and reviewers
- Reviews with line comments, a summary and a vote, replies to remarks, resolved threads shown and resolved from Squadron, merges, and title and description changes
- Pipeline builds of each project, with a notification when one ends
- Builds run on a pull request show its number and title and are filed under PR checks
- Chips for Azure DevOps pull request links in conversations
- Azure DevOps accounts with an organization and a personal access token checked before it is saved, one per organization
