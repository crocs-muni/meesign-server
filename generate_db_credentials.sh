#!/usr/bin/env bash

set -e

if ! command -v openssl >/dev/null 2>&1; then
    echo "The 'openssl' command is not available. Please, install OpenSSL utility."
    exit 1
fi

ENV_FILE='./.env'
ENV_FILE_BACKUP="$ENV_FILE.bak"
FORCE_FLAG='--force'

if test -f "$ENV_FILE" && test "$1" != "$FORCE_FLAG"; then
    echo "The '$ENV_FILE' file exists and the '$FORCE_FLAG' command line flag was not used."
    echo "Doing nothing."
    exit 0
fi

echo "Archiving previous '$ENV_FILE' in '$ENV_FILE_BACKUP'."
mv "$ENV_FILE" "$ENV_FILE_BACKUP"

echo "Storing new database credentails in '$ENV_FILE'."
# create a new file with 600 permissions
install --mode 600 /dev/null "$ENV_FILE"
# store db credentials with freshly generated password
cat <<EOF > "$ENV_FILE"
POSTGRES_USER="meesign"
POSTGRES_PASSWORD="$(openssl rand -hex 32)"
POSTGRES_DB="meesign"
DATABASE_URL="postgres://\${POSTGRES_USER}:\${POSTGRES_PASSWORD}@localhost:\${POSTGRES_PORT}/\${POSTGRES_DB}"
EOF
