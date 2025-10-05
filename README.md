# Alerting – Zabbix → Notifications Linux (Ack/Unack)

Petit binaire Rust qui récupère les **problèmes Zabbix** via l’API JSON-RPC et affiche des **notifications système** (Linux Mint/Cinnamon, libnotify).
Chaque toast peut **ouvrir Zabbix**, **acknowledge** ou **unacknowledge** le problème, avec **message optionnel**.

---

## ✨ Fonctionnalités

* Récupération des problèmes actifs (`problem.get`) avec filtre **ACK/UNACK/ALL**
* Résolution du **nom d’hôte** lié à chaque événement
* **Notifications** avec niveau d’urgence selon la sévérité Zabbix
* **Boutons intégrés** :

  * **Ouvrir** dans Zabbix (URL paramétrable), ne fonctionne pas quand Firefox est navigateur web par défaut, mais ok Brave
  * **Ack** / **Unack** (appelle `event.acknowledge`)
  * **Message** facultatif saisi via `zenity --entry`
* Sous **Windows 11**, un bouton **Valider** permet l’ack direct (commentaire optionnel) depuis le toast.
  Si l’icône (`notify.icon`) est introuvable, Windows retombe sur la tuile générique "New notification".
* Concurrency contrôlée pour les appels API
* **Configuration** par fichier TOML + variables d’environnement (ENV > fichier > défauts)

---

## ⚙️ Prérequis

* Linux (Mint 22 recommandé)
* Zabbix >= 5.x (API JSON-RPC active, **token** API)
* Paquets système :

  ```bash
  sudo apt-get update
  sudo apt-get install -y xdg-utils zenity  # libnotify est déjà présent sur Mint
  ```

---

## 🚀 Build & installation

```bash
# Build release
cargo build --release

# Installer le binaire
sudo install -Dm755 target/release/alerting /usr/local/bin/alerting
```

---

## 🔧 Configuration

Le binaire charge, par ordre de priorité : **ENV** > **fichier** > **valeurs par défaut**.

* **Fichier par défaut** : `CONFIG_FILE` (si défini) sinon `config.toml` dans le cwd.
  Recommandé : `~/.config/alerting/config.toml`

```toml
# ~/.config/alerting/config.toml
[zabbix]
url = "https://zabbix.example.com/api_jsonrpc.php"
# token = "xxxxxxxx..."              # ou via ENV ZBX_TOKEN
limit = 20
concurrency = 8
ack_filter = "unack"                 # "unack" | "ack" | "all"
open_url_fmt = "https://zabbix.example.com/zabbix.php?action=problem.view&filter_eventid={eventid}"

[notify]
appname = "Check Agent"
sticky = false
timeout_ms = 8000
timeout_default = false
open_label = "Ouvrir dans Zabbix"
notify_acked = false                 # cacher les problèmes déjà ACK

[app]
max_notif = 5
```

### Variables d’environnement

| Nom                      | Rôle                                                 | Ex.                                  |
| ------------------------ | ---------------------------------------------------- | ------------------------------------ |
| `ZBX_URL`                | Endpoint JSON-RPC Zabbix                             | `https://…/api_jsonrpc.php`          |
| `ZBX_TOKEN`              | **Token API** (prioritaire sur le TOML)              | `xxxxxxxx…`                          |
| `LIMIT`                  | Nombre max de problèmes à récupérer                  | `20`                                 |
| `CONCURRENCY`            | Appels parallèles pour la résolution d’hôtes         | `8`                                  |
| `ACK_FILTER`             | Filtre de récupération : `unack` / `ack` / `all`     | `unack`                              |
| `MAX_NOTIF`              | Nombre max de toasts affichés                        | `5`                                  |
| `NOTIFY_APPNAME`         | Nom d’application des toasts                         | `Innlog Agent`                       |
| `NOTIFY_STICKY`          | `true` = toasts persistants                          | `false`                              |
| `NOTIFY_TIMEOUT_MS`      | Timeout custom des toasts (ms)                       | `8000`                               |
| `NOTIFY_TIMEOUT_DEFAULT` | Utiliser le timeout par défaut du système            | `false`                              |
| `NOTIFY_ICON`            | Chemin d’icône                                       | `/path/icon.png`                     |
| `NOTIFY_OPEN_LABEL`      | Libellé du bouton « Ouvrir »                         | `Ouvrir dans Zabbix`                 |
| `NOTIFY_ACKED`           | Afficher aussi les problèmes déjà ACK                | `false`                              |
| `ZBX_OPEN_URL_FMT`       | URL « Ouvrir » (placeholder `{eventid}` obligatoire) | `https://…&filter_eventid={eventid}` |
| `CONFIG_FILE`            | Chemin du fichier TOML                               | `~/.config/alerting/config.toml`     |

---

## ▶️ Exécution

### Premier test (foreground)

```bash
CONFIG_FILE=~/.config/alerting/config.toml ZBX_TOKEN=xxxxxxxx /usr/local/bin/alerting
```

### Service systemd **utilisateur** (recommandé)

```bash
mkdir -p ~/.config/alerting ~/.config/systemd/user
chmod 700 ~/.config/alerting

# Option : token séparé
cat >~/.config/alerting/alerting.env <<'ENV'
ZBX_TOKEN=xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx
ENV
chmod 600 ~/.config/alerting/alerting.env

cat >~/.config/systemd/user/alerting.service <<'UNIT'
[Unit]
Description=Alerting (Zabbix toaster + Ack/Unack)
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
ExecStart=/usr/local/bin/alerting
Environment=CONFIG_FILE=%h/.config/alerting/config.toml
EnvironmentFile=%h/.config/alerting/alerting.env
Restart=always
RestartSec=5
NoNewPrivileges=yes
ProtectSystem=full
PrivateTmp=yes
LockPersonality=yes
MemoryDenyWriteExecute=yes

[Install]
WantedBy=default.target
UNIT

systemctl --user daemon-reload
systemctl --user enable --now alerting.service
journalctl --user -u alerting -f
```

> **Pourquoi “user service” ?** Les toasts et boutons Ack/Unack utilisent le **bus D-Bus de session** (notifications interactives). Un service “system” ne verrait pas ta session graphique.

---

## 🪟 Windows 11

* Build : `rustup target add x86_64-pc-windows-msvc` puis `cargo build --release --target x86_64-pc-windows-msvc`.
* Config : `config.toml` peut être placé dans `%APPDATA%\alerting\config.toml` (mêmes clés que la version Linux).
* Lancer : `target\x86_64-pc-windows-msvc\release\alerting.exe` depuis un terminal PowerShell.
* Interaction : bouton **Valider** intégré pour acquitter l’alerte (commentaire optionnel saisi dans le toast).
  Les toasts utilisent l’icône configurée (`notify.icon`). Si ce chemin est invalide, la notification générique Windows (titre "New notification") est affichée.
* Limitations actuelles : le bouton « Ouvrir » reste inactif sur Windows, les toasts ne proposent pas d’Unack.
* Pour l’exécution au démarrage, créer une tâche planifiée (Task Scheduler) pointant vers `alerting.exe` avec `Start in` défini sur le dossier de config.

---

## 🖱️ Interaction des toasts

* **Ouvrir** : lance `xdg-open` avec l’URL construite via `open_url_fmt` (remplacement `{eventid}`).
* **Ack** : appelle `event.acknowledge` avec **bitmask** `2` (et `+4` si message saisi).
* **Unack** : bitmask `16` (et `+4` si message saisi).
* **Message optionnel** : si activé dans le binaire, une boîte `zenity --entry` s’ouvre.
  Si vide/annulé/`zenity` absent → envoi **sans message**.

---

## 🔍 Dépannage rapide

* **Parse error (JSON)** côté Zabbix : généralement un problème d’échappement si tu construis du JSON à la main. Le binaire sérialise proprement, mais vérifie l’URL API et le **token**.
* **Pas de notifications** : vérifier la session (test `notify-send "test"`), et que le service tourne en mode **user**.
* **Boutons inactifs** : vérifier `xdg-utils` et la présence de `zenity` (facultatif mais conseillé).
* **403/permission** : l’utilisateur API doit avoir les droits **Read/Write** sur les objets visés (Ack/Unack).

---

## 🧱 Sécurité

* Stocke le **token** dans `~/.config/alerting/alerting.env` (600) ou via un gestionnaire de secrets.
* Le service applique des options systemd de confinement raisonnables, compatibles avec l’UI.

---

## 🧪 Dév

```bash
cargo fmt
cargo clippy -- -D warnings
cargo run
```

---

## 📄 Licence

Voir `LICENSE` dans le dépôt.

## Disclaimer

Le logiciel vient tel quel sans garanties.
