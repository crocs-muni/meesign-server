# Architecture overview
This repository contains a gRPC server which coordinates multi-party threshold protocols.

## Terminology
- A **client** is a regular client in the gRPC sense.
- A **device** is a **client** with an issued certificate. In some parts of the code, it might mean one **share** of a **participant** because those used to be synonymous.
- A **task** is an abstracted communication among multiple **participants** with multiple phases and a result if it completes successfully.
- A **participant** is a **device** and its **shares** within a **group** or a **task**.
- A **share** is a unit of computation and voting power in the protocol layer and on the client side.
- A **threshold** is the minimum number of accepting **shares** needed to start a **task**. It is mostly a property of a **group**, but is also used in the context of a **task**:
  - The **threshold** of a **group task** is the threshold of its resulting **group**.
  - The **threshold** of a **threshold task** is the threshold of the **group** within which it runs.
- A **voting task** is the first task phase after a **task**'s creation where the **task participants** can either accept or reject the **task**. A participant stays a participant even if it rejects the task, it receives the task result if the task finishes successfully.
- A **declined task** is a task phase which a **voting task** enters immediately once it's impossible to gather enough accepting votes.
- A **running task** is a task phase which a **voting task** enters once it gathers enough accepting votes and a **communicator** is created. It gives control to a **protocol**.
- A **failed task** is a task phase which is entered upon certain failures.
- A **finished task** is a task phase which a **running task** enters once its **protocol** successfully computes a result.
- A **communicator** is a bridge between **devices** and a **protocol**. It handles **message** gathering, relaying, broadcasting and **protocol** result collection.
- A **message** is serialized data exchanged among **active shares**. All messages pass through the server. Depending on the origin, they are categorized into **client** and **server** messages. Relayed messages are also bundled into **server messages**.
- A **group task** is a task which establishes a **group**. All its **participants** need to accept in order for the task to start.
- A **group** is an abstraction over a shared key and a **threshold**.
- A **threshold task** is a task which needs a **group** to be created and is an umbrella term for **sign**, **sign pdf** and **decrypt tasks**. In order for a threshold task to start, at least the  **group**'s **threshold** number of **participants** need to accept.
- A **protocol** is an abstraction for actual multi-party threshold protocols.
- An **active share** is a **share** which participates in the computation of a **protocol**.
- A **protocol index** is the index of an **active share** and corresponds to the way **protocols** manage their active parties. The assignment of indices to shares is rather complicated, see [*Protocol index assignment*](#protocol-index-assignment).

## Module structure
- `persistence` contains code related to the server persistence.
- `state` contains and manages all of the server's state.
- `interfaces` contains the modules `grpc` and `timer`, which define long-running services
  - `grpc` provides the server's gRPC endpoints, handles client registration and certificates
  - `timer` periodically runs checks over the state
- `task_store` manages the persistence and caching of tasks
- `task` contains the logic for task computation
- `protocol` contains the logic for protocol computation
- `communicator` defines the communicator
- `error` contains definitions of error variants
- `utils` contains a few helper functions

## Persistence
Most of the server state is persisted throughout server restarts, but some state is deliberately ephemeral and kept only in the RAM. The ephemeral state is mostly data which changes "rapidly", namely activity timestamps and messages exchanged during protocol computation.

Persistence is handled in the `state` module, with the exception of `task_store`, which is only used within `state`. This is to decouple the logic from bookkeeping.

The `persistence` module is supposed to be a "dumb" interface for communicating with the DB. In particular, it shouldn't validate data, perform complex logic, etc...

## State machines and state changes
Much of the actual logic can be easily modeled using state machines. We use typestates to enforce valid state transitions. For example, a running task cannot change into a voting task.

Functions which update some state return a state change enum, which enforces handling of all possible situations and explicitly defines the logic. For example, saving a vote in a voting task can have three outcomes:
- The task is accepted and transitions into a running task
- The task is declined and transitions into a declined task
- The task does not have enough votes to determine an outcome and it stays as a voting task

## Protocol index assignment
Multiple shares per device were implemented in an ad-hoc way and devices do not understand share indices. Instead, they accept a vector of *k* messages, one per active share, and they implicitly assign indices *[0..(k-1)]* to the messages. The index assignment algorithm thus has to more or less work like this:
1. Gather all candidate shares sorted by their corresponding device id - this lets us deterministically recover correct indices without persisting the index assignment.
2. Assign indices *[0..n]* to the sorted shares.
3. For each device, get the range of indices assigned to its shares.
4. Choose the active shares such that for each device, they are chosen from the start of its range.

For example, consider a `3-of-1,2,3` setup with devices `A`, `B`, `C`.
1. Gather sorted candidate shares: `[A, B, B, C, C, C]`
2. Assign indices: `{0: A, 1: B, 2: B, 3: C, 4: C, 5: C}`
3. Get the index ranges per device: `{A: [0..0], B: [1..2], C: [3..5]}`
4. Choose 3 active shares from range beginnings: `{1: B, 3: C, 4: C}`

==> The protocol indices are thus `[1, 3, 4]`.

# Guides for common changes
This section provides guides for certain changes to the codebase which may be common.

## Adding a new protocol type
Adding new protocols must be coordinated with the `meesign-crypto` repository.

Protocols are defined throughout several places in the codebase:
1. The `proto/meesign.proto` files in this and `meesign-crypto` repositories define a `ProtocolType` enumeration. Both must be extended.
2. A few trait implementations for `ProtocolType` must be extended in `persistence/enums.rs`.
3. The `protocol_type` enum must be extended in the DB migrations.
4. A module should be added into `protocols`, similar to other protocols defined there.
5. The module needs to create a type implementing the `Protocol` trait for each of its variants, for example `<protocol>Group`, `<protocol>Sign`, ...
6. The module should use constants from `meesign_crypto::protocols::<protocol>`.

The overall structure should reflect the way other protocols are implemented. See the `protocols/frost.rs` module for example.

## Adding a new task type
Adding new task types must be coordinated with the `meesign-client` repository.

If the task follows the usual task phases (voting, declined, running, failed, finished), then it should follow the structure of other task types already established in this repo. Otherwise, it must be handled exceptionally.

Here is a general process for when the new task type follows the usual task phases:
1. The `proto/meesign.proto` file defines a `TaskType` enumeration. It must be extended.
2. A few trait implementations for `TaskType` must be extended in `persistence/enums.rs`.
3. The `task_type` enum must be extended in the DB migrations.
4. The `RunningTaskContext` enum in `tasks/mod.rs` must be extended.
5. A module should be added into `tasks`, similar to other tasks defined there.
6. The module needs to create a type implementing the `RunningTask` trait.

The overall structure should reflect the way other tasks are implemented. See the `tasks/sign.rs` module for example.
