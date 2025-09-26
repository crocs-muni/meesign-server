# MeeSign Server

Server-side implementation for the MeeSign system.

## Usage

The MeeSign server requires PostgreSQL database instance. There are several
ways of running the server and the database instance, for most of which we use
Docker and run the components inside dedicated containers. However, customized
deployment scenarios are possible.

### Published Docker containers

If you have Docker installed, you can start both the MeeSign Server and the
PostgreSQL database as Docker containers using Docker Compose.

1. First, you need to generate the private keys and certificates for the
   MeeSign server and the credentials for the Postgres database:

    ```bash
    bash generate_keys.sh
    bash generate_db_credentials.sh

2. Then you can start the Docker services, 

    ```bash
    docker compose --file compose.base.yaml --file compose.deploy.yaml  up --detach
    ```

    In case the `up` sub-command fails, try cleaning up the previous containers
    with (and try bringing it `up` again):

    ```bash
    docker compose -f compose.base.yaml -f compose.deploy.yaml down --volumes --remove-orphans
    docker compose -f compose.base.yaml -f compose.deploy.yaml  up --detach
    ```
    The Docker image for MeeSign Server is pulled from the [Docker
    hub](https://hub.docker.com/r/crocsmuni/meesign). In `compose.deploy.yaml` you can
    specify whether to use `latest` (or other stable version), or `nighlty`
    (the bleeding edge, but unstable) version of the `crocsmuni/meesign` Docker
    image. If you wish to start the MeeSign server from the local sources, you
    need to update the `meesign-server` service in `compose.deploy.yaml` with:

    ```yaml
    services:
      ... 
      meesign-server:
        build:
          context: .
          dockerfile: Dockerfile
    ```

    For testing local sources, see **Integration Tests** section below.

Once up, the MeeSign should be available on `localhost` on port `1337`. The
data stored in the database container should be accessible outside the
containers. For example, on Linux look for (root privileges might be required):

    ```bash
    ls /var/lib/Docker/volumes/meesign-deploy_db-data/
    base          pg_dynshmem    pg_logical    pg_replslot   pg_stat      pg_tblspc    pg_wal                postgresql.conf
    global        pg_hba.conf    pg_multixact  pg_serial     pg_stat_tmp  pg_twophase  pg_xact               postmaster.opts
    pg_commit_ts  pg_ident.conf  pg_notify     pg_snapshots  pg_subtrans  PG_VERSION   postgresql.auto.conf  postmaster.pid
    ```

### Build from source

MeeSign server itself can also be built directly from source.

1. [Install Rust](https://www.rust-lang.org/tools/install) and OpenSSL.

2. Clone this repository and navigate to the project:

   ```bash
   git clone https://github.com/crocs-muni/meesign-server
   cd meesign-server
   ```

3. Generate private keys and certificates and store them in `./keys` directory:

   ```bash
   bash generate_keys.sh
   ```

4. [Prepare MeeSignHelper](https://github.com/dufkan/meesign-helper) to support PDF signing.

Now, the MeeSign server binary can be built with `cargo build`. In order to run
the server, a Postgres database is required. If you want to use custom Postgres
database, it's enough to set `DATABASE_URL` enviromental variable before
starting the server. For example:

    ```text
    export DATABASE_URL="postgres://user:password@localhost:5432/database"
    ```

Otherwise, you can start a Postgres database in a Docker cotainer.

5. Generate the PostgreSQL database connection credentials and store them in
   the `./.env` file. You can use `--force` to regenerate the credentials:

   ```bash
   bash generate_db_credentials.sh
   ```

6. Start PostgreSQL database server in a Docker container. You can use
   different versions by editing `postgres:{version}` (e.g., `postgres:latest`
   for the latest build):

   ```bash
   docker run --env-file ./.env --restart always --name meesign-postgres --user postgres --publish 5432:5432 postgres:17
   ```

7. Build and run the server (use `--release` for optimized build):

   ```bash
   cargo run
   ```

### Development and tests

There are three layers of tests available. Standard Rust unit tests, database
tests, and integration tests (without the GUI).

#### Unit tests

To execute unit tests (excluding tests that require database), simply run `cargo test`.

#### Database tests

Database tests require an instance of PostgresSQL database. To simplify the
database tests setup and achieve isolation of individual tests, we are using
[Testcontainers](https://testcontainers.com/), in particular, the Rust
[modules](https://github.com/testcontainers/testcontainers-rs-modules-community/).
Each `db-tests` has access to a ephemeral PostgresSQL database in an ephemeral
Docker container. The database tests are _hidden_ behind a feature flag
`db-tests`, that otherwise has no usage. Database tests cannot be executed
without the other unit tests. Thus, to run all tests do:

    ```bash
    $ cargo run --features db-tests
    ```

In case you would like to test against another database in tests, you can set
the enviromental variable `TEST_DATABASE_URL`, for example:

    ```bash
    export TEST_DATABASE_URL=postgres://meesign:mysecretpassword@localhost:5433/meesign
    ```

#### Integration tests

Integration tests of MeeSign Server against the [MeeSign
Client](https://github.com/crocs-muni/meesign-client.git). For integration
tests, static keys and certificates in `integration-tests/keys` and static
database credentials in `integration-tests/.env` are used. There is a
convenience script `run_integration_tests.sh`. When executed without any
parameters, all of the [client
tests](https://github.com/crocs-muni/meesign-client/blob/main/meesign_core/test/core_test.dart)
are executed.

    ```bash
    ./run_integration_tests.sh
    ```

In case the test containers don't boot up properly, for example with the following error

    ```text
    Error response from daemon: network {some-hash} not found
    ```
then you can try to bring all test containers down, and try again

    ```bash
    ./run_integration_tests.sh down
    ./run_integration_tests.sh
    ```

Integration tests filtering is possible with [Test Path
Queries](https://pub.dev/packages/test#test-path-queries), similarly to when
calling Dart test directly. For example, to call 2-out-of-3 group setting for
[signing
PDF](https://github.com/crocs-muni/meesign-client/blob/main/meesign_core/test/core_test.dart#L220)
and challenge
([GG18](https://github.com/crocs-muni/meesign-client/blob/main/meesign_core/test/core_test.dart#L226C26-L228)
and
[FROST](https://github.com/crocs-muni/meesign-client/blob/main/meesign_core/test/core_test.dart#L240-L241)),
run:

    ```bash
    $ ./run_integration_tests.sh --name "sign|challenge" --name "2-3"
    ```

The `run_integration_tests.sh` is just a thin wrapper around `docker compose`.
Docker compose provides more options, which might be needed when running
integration tests. In particular, the `docker compose config` sub-command is
handy when debugging the overriding the `./compose.base.yam` with
`./integration-tests/compose.yaml`. Especially, be aware of the [enviromental
variables
precedence](https://docs.docker.com/compose/how-tos/environment-variables/envvars-precedence/)
in Docker Compose.

## Acknowledgements

* This work was supported by the Ministry of the Interior of the Czech Republic under grant VJ01010084 in program IMPAKT I.
