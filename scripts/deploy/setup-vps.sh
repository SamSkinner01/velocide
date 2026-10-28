#!/usr/bin/env bash
# One-time setup of a VPS (Ubuntu or Debian) for VELOCIDE's online service, and optionally the
# dedicated server, with deploys from CI (docs/rendezvous.md "Deploying"). As root, from a copy
# of this folder:
#
#   bash setup-vps.sh --deploy-key "ssh-ed25519 AAAA... velocide-deploy" [--with-server]
#
# It creates the user velocide-deploy, whose key can only run velocide-deploy (install a
# release from GitHub), installs the systemd units (each service runs as its own throwaway
# user, sandboxed), opens the UDP ports in ufw if ufw is on, installs the newest release and
# starts the services. Running it again updates the units and the deploy script, and replaces
# the deploy key.
#
# Root never runs, reads into or re-owns anything under /opt/velocide/deploy (the deploy user
# owns it, so root treats it as untrusted): it only creates what is missing, refuses symlinks
# on the way, and checks the service through the deploy script as the deploy user.
set -euo pipefail
cd "$(dirname "$0")"

die() {
    echo "setup-vps: $*" >&2
    exit 1
}

key=
with_server=false
while [ $# -gt 0 ]; do
    case "$1" in
    --deploy-key)
        key=${2:-}
        shift 2
        ;;
    --with-server)
        with_server=true
        shift
        ;;
    *) die "unknown option '$1' (usage: setup-vps.sh --deploy-key 'ssh-ed25519 ...' [--with-server])" ;;
    esac
done

[ "$(id -u)" = 0 ] || die "run as root"
# One public key, nothing that could add options to its authorized_keys line.
[[ "$key" =~ ^(ssh-ed25519|ecdsa-sha2-nistp256|ecdsa-sha2-nistp384|ssh-rsa)\ [A-Za-z0-9+/]+=*(\ [A-Za-z0-9@._-]+)?$ ]] ||
    die "--deploy-key needs one public key line, e.g. \"ssh-ed25519 AAAA... velocide-deploy\""
for tool in curl python3 flock sha256sum systemctl runuser; do
    command -v "$tool" >/dev/null || die "$tool is missing (apt-get install curl python3 util-linux coreutils)"
done

# A directory root creates or uses: refused if something put a symlink there.
no_link() {
    [ ! -L "$1" ] || die "$1 is a symlink: remove it (and find out who made it) first"
}

home=/var/lib/velocide-deploy
for path in /opt/velocide /opt/velocide/deploy /opt/velocide/deploy/releases \
    "$home" "$home/.ssh" "$home/.ssh/authorized_keys"; do
    no_link "$path"
done
if ! id velocide-deploy >/dev/null 2>&1; then
    useradd --system --no-create-home --home-dir "$home" --shell /bin/sh velocide-deploy
fi
passwd -l velocide-deploy >/dev/null
# Its home and keys belong to root: the deploy user can't change what its key may do.
install -d -o root -g root -m 755 "$home" "$home/.ssh"
printf 'restrict,command="/usr/local/bin/velocide-deploy" %s\n' "$key" >"$home/.ssh/authorized_keys"
chown root:root "$home/.ssh/authorized_keys"
chmod 644 "$home/.ssh/authorized_keys"

install -d -o root -g root -m 755 /opt/velocide
# Made once; never chowned again by root (it belongs to the deploy user from then on).
for dir in /opt/velocide/deploy /opt/velocide/deploy/releases; do
    [ -d "$dir" ] || install -d -o velocide-deploy -g velocide-deploy -m 755 "$dir"
done
install -o root -g root -m 755 velocide-deploy /usr/local/bin/velocide-deploy

install -o root -g root -m 644 velocide-rendezvous.service /etc/systemd/system/
if $with_server; then
    install -o root -g root -m 644 velocide.service /etc/systemd/system/
fi
systemctl daemon-reload

echo "==> installing the newest release"
runuser -u velocide-deploy -- /usr/local/bin/velocide-deploy latest

systemctl enable --now velocide-rendezvous.service
ports=(47400/udp)
if $with_server; then
    systemctl enable --now velocide.service
    ports+=(47420/udp)
fi
if command -v ufw >/dev/null && ufw status | grep -q '^Status: active'; then
    for p in "${ports[@]}"; do ufw allow "$p"; done
fi

sleep 3
if version=$(runuser -u velocide-deploy -- /usr/local/bin/velocide-deploy probe); then
    echo "==> the online service answers on UDP 47400: $version"
else
    echo "warning: the online service doesn't answer yet: journalctl -u velocide-rendezvous" >&2
fi
cat <<EOF

Next (docs/rendezvous.md "Deploying"):
  - Open UDP ${ports[*]} in your provider's firewall, if it has one.
  - DNS: an A record for the service's name (DNS only: a proxy like Cloudflare's doesn't
    carry UDP).
  - GitHub repository variables: VELOCIDE_RENDEZVOUS=<name>:47400 and DEPLOY_HOST=<name or IP>.
  - GitHub secrets (environment "production"): DEPLOY_SSH_KEY (the private key) and
    DEPLOY_KNOWN_HOSTS, the output of: ssh-keyscan -t ed25519 <DEPLOY_HOST>
EOF
