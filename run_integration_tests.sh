#!/usr/bin/env bash

set -e

if test "$1" == "down"; then
    docker compose \
        --file compose.base.yaml \
        --file integration-tests/compose.yaml \
        --env-file integration-tests/.env \
        down \
            --volumes \
            --remove-orphans
    exit "$?"
fi

if [ -z "$1" ]; then
    docker compose \
        --file compose.base.yaml \
        --file integration-tests/compose.yaml \
        --env-file integration-tests/.env \
        up \
            --abort-on-container-exit \
            --exit-code-from \
            test-client
else
    # If any command line arguments are supplied, these are pass on to the
    # `test-client`, i.e. calling `dart test --reporter=expanded "$@"`
    docker compose \
        --file compose.base.yaml \
        --file integration-tests/compose.yaml \
        --env-file integration-tests/.env \
        run \
            --rm \
            test-client \
            --disable-analytics \
            test --reporter=expanded \
            "$@"
fi
