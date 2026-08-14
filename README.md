# LFSX

Serveur **Git LFS** auto-hébergé, écrit en Rust. Il stocke les gros fichiers binaires d'un dépôt
(assets de jeu, textures, modèles, vidéos) sans consommer le quota LFS de l'hébergeur. Le dépôt
Git reste où il est — seul le transfert LFS est redirigé.

## Utilisation côté client

Dans le dépôt concerné, commiter un `.lfsconfig` à la racine :

```
[lfs]
	url = https://lfs.exemple.fr/FerrLabs/mon-projet
```

Les deux derniers segments sont l'organisation et le projet ; ils délimitent l'espace de stockage.
`git lfs install` doit avoir été exécuté avant le clone, sinon les fichiers arrivent sous forme
de pointeurs de 130 octets.

## Configuration

| Variable | Défaut | Rôle |
|---|---|---|
| `LFSX_BIND` | `0.0.0.0:8080` | adresse d'écoute |
| `LFSX_STORAGE_ROOT` | `/var/lib/lfsx` | racine du stockage des objets |
| `LFSX_PUBLIC_URL` | `http://<bind>` | URL publique, utilisée pour construire les liens de transfert |

`LFSX_PUBLIC_URL` doit correspondre à l'URL réellement vue par le client : c'est elle qui est
renvoyée dans la réponse batch, et le client s'y reconnecte pour chaque objet.

## API

Le protocole est petit — quatre routes :

- `POST /{org}/{repo}/objects/batch` — négociation : le client annonce ses objets, le serveur
  répond par objet avec un lien d'upload ou de download
- `PUT /{org}/{repo}/objects/{oid}` — dépôt d'un objet
- `GET /{org}/{repo}/objects/{oid}` — récupération
- `POST /{org}/{repo}/objects/verify` — vérification après dépôt

L'API de verrous (`locks`) n'est pas implémentée : git-lfs la sonde, constate son absence et
bascule seul sur `lfs.locksverify false`.

## Garanties

- **Vérification d'intégrité** : le SHA256 du contenu reçu est recalculé en flux et comparé à
  l'identifiant annoncé. Un objet dont le contenu ne correspond pas est rejeté et rien n'est
  conservé.
- **Écriture atomique** : le transfert va dans un fichier temporaire, renommé seulement après
  validation. Un transfert interrompu ne laisse pas d'objet corrompu derrière lui.
- **Flux de bout en bout** : ni l'upload ni le download ne chargent le fichier en mémoire, ce qui
  permet des objets de plusieurs gigaoctets.

## Ce qui n'est pas encore fait

**L'authentification.** Le serveur accepte pour l'instant toutes les requêtes ; il ne doit pas
être exposé publiquement en l'état.

Une contrainte à connaître avant de l'implémenter, parce qu'elle n'est pas évidente et qu'elle
condamne l'approche la plus naturelle : **on ne peut pas mettre l'authentification dans un reverse
proxy**. La réponse batch renvoie les URL de transfert d'objets ; si le serveur les annonce comme
déjà authentifiées (`"authenticated": true`) sans fournir d'en-tête, git-lfs appelle ces URL sans
identifiants et boucle sur des `401`. C'est le défaut qui rend rudolfs inutilisable derrière un
BasicAuth Traefik. LFSX n'émet jamais `authenticated`, donc le client authentifie lui-même chaque
transfert — un test le verrouille (`batch_never_claims_the_transfer_is_pre_authenticated`).

L'authentification doit donc vivre dans le serveur. La piste retenue est de calquer les
permissions sur celles du dépôt Git distant : le client présente le même token que pour cloner en
HTTPS, le serveur l'utilise pour vérifier ses droits sur le dépôt, et en dérive lecture ou
écriture. Aucun compte à gérer.

## Développement

```bash
cargo test
```
