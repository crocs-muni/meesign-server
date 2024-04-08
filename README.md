# MeeSign Server

Server-side implementation for MeeSign system.

## Usage

### Local Build

1. [Install Rust](https://www.rust-lang.org/tools/install)

2. Clone the repository:

   ```bash
   git clone https://github.com/crocs-muni/meesign-server
   ```

3. Generate private keys and certificates:

   ```bash
   bash generate_keys.sh
   ```

4. [Prepare MeeSignHelper](https://github.com/dufkan/meesign-helper)

5. Set up PostgreSQL server

   ```bash
   docker run --restart always --name meesign-postgres --user postgres -e POSTGRES_USER=meesign -e POSTGRES_PASSWORD=mysecretpassword -e POSTGRES_DB=meesign --detach --publish 5432:5432 postgres
   ```

6. Store the connection string

   ```bash
   echo DATABASE_URL=postgres://meesign:mysecretpassword@localhost/meesign >> .env
   ```

7. Build and run the server:

   ```bash
   cargo run
   ```

### Run in a Docker Container

1. Generate private keys and certificates:

   ```bash
   bash generate_keys.sh
   ```

2. Run the production docker-compose:

   ```bash
   docker-compose up --detach
   ```

   **NOTE:** There are 2 types of available releases:

   1. **latest** - this is the latest stable version, you can optionally specify a specific stable version
   2. **nightly** - a bleeding-edge unstable version that is released every midnight

## Development

1. Run the development docker-compose:

   ```bash
   docker-compose --file ./docker-compose.dev.yaml up --detach
   ```

2. Create a development env file with a connection URL. This URL should point to the development/user testing instance:

   ```bash
   echo DATABASE_URL=postgres://meesign:mysecretpassword@localhost/meesign >> .env
   ```

3. Tests expect an empty database, though they don't commit any changes. Variable _TEST_DATABASE_URL_ is used in such cases.

   ```bash
   echo TEST_DATABASE_URL=postgres://meesign:mysecretpassword@localhost:5433/meesign >> .env
   ```

   There are 2 types of available releases:
   1. **latest** - this is the latest stable version, you can optionally specify a specific stable version
   2. **nightly** - a bleeding-edge unstable version that is released every midnight

## Acknowledgements

* This work was supported by the Ministry of the Interior of the Czech Republic under grant VJ01010084 in program IMPAKT I.
