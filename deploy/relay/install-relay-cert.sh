#!/bin/sh
# certbot runs this each time it gets or renews the relay's certificate
# (a "deploy hook"): it copies the new files to where the relay reads them
# and restarts the relay, which reads its certificate only when it starts.
# Installed as /etc/letsencrypt/renewal-hooks/deploy/wanderbooks-relay.sh,
# mode 0755. certbot sets RENEWED_LINEAGE to the certificate's folder.
set -eu
dest=/etc/wanderbooks-relay
install -o root -g wanderbooks-relay -m 0644 "$RENEWED_LINEAGE/fullchain.pem" "$dest/relay-cert.pem"
install -o root -g wanderbooks-relay -m 0640 "$RENEWED_LINEAGE/privkey.pem" "$dest/relay-key.pem"
systemctl restart wanderbooks-relay
