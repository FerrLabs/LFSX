# LFSX

Serveur Git LFS auto-hébergé, Rust + axum. Workspace Cargo, un seul crate : `server/`
(binaire `lfsx-server`).

## Découpage

```
server/src/
  model.rs     # types du protocole batch (requête, réponse, actions, erreurs)
  storage.rs   # LocalStore : chemins, écriture en flux + vérification, lecture
  routes.rs    # handlers axum et câblage du routeur
  config.rs    # variables d'environnement et construction des URL publiques
  error.rs     # erreurs du domaine et mapping vers les codes HTTP
  lib.rs       # app(config) -> Router
  main.rs      # bootstrap
server/tests/api.rs
```

## Invariants à ne pas casser

- **Ne jamais émettre `authenticated` dans la réponse batch.** Annoncer
  `"authenticated": true` sans fournir d'en-tête fait envoyer les transferts sans identifiants,
  et le client boucle sur des 401. C'est le défaut qui rend rudolfs inutilisable derrière un
  reverse proxy authentifiant. Un test verrouille ce comportement.
- **Le SHA256 est recalculé en flux à chaque dépôt** et comparé à l'oid annoncé. Un contenu qui
  ne correspond pas est rejeté sans rien laisser sur le disque.
- **Écriture atomique** : fichier temporaire puis `rename`, jamais d'écriture directe à
  l'emplacement final.
- **Rien ne doit être chargé en mémoire** : upload et download passent en flux, les objets font
  couramment plusieurs gigaoctets.

## Conventions

Voir le CLAUDE.md du workspace parent. En résumé : pas de commentaires explicatifs, code
idiomatique Rust, SRP par fichier, YAGNI. Commits en Conventional Commits sur une ligne.

## Tests

`cargo test`. Les tests d'intégration montent le routeur sur un `tempdir` et passent par
`tower::ServiceExt::oneshot`. Tester le comportement et les chemins d'erreur, pas la présence
des routes.
