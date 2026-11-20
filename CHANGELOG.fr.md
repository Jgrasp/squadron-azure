# Journal des modifications

## 0.3.0 · 2026-11-20

- Ouvrez une pull request depuis la branche poussée d’un agent, en brouillon si vous le souhaitez, avec le bouton Ouvrir une pull request de Squadron
- Une description plus longue que les 4 000 caractères acceptés par Azure DevOps est coupée à 4 000
- Une branche qui a déjà une pull request active vers la même base reçoit un message clair

## 0.2.0 · 2026-11-20

- La fusion d’une pull request propose les types de fusion qu’autorise sa branche cible : merge, squash, ou rebase et fast-forward
- La fusion peut supprimer la branche source, comme le prévoient les options de complétion de la pull request, sauf si des stratégies de branche la protègent

## 0.1.0 · 2026-11-20

- Les pull requests de vos projets Azure DevOps dans Squadron : celle de la branche de chaque agent, celles à relire, les vôtres et les autres, avec leurs checks et leurs relecteurs
- Des revues avec commentaires de ligne, résumé et vote, des réponses aux remarques, des fils résolus affichés et résolus depuis Squadron, des fusions, et le changement du titre et de la description
- Les builds de pipelines de chaque projet, avec une notification à la fin de chacun
- Les builds lancés sur une pull request affichent son numéro et son titre et sont rangés dans les checks de PR
- Des puces pour les liens de pull requests Azure DevOps dans les conversations
- Des comptes Azure DevOps avec une organisation et un jeton d’accès personnel vérifié avant d’être enregistré, un par organisation
