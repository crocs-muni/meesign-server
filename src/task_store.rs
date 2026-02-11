use crate::error::Error;
use crate::tasks::{Task, VotingTask};
use async_trait::async_trait;
use std::ops::{Deref, DerefMut};
use uuid::Uuid;

#[async_trait]
pub trait TaskStore {
    type TaskRef: Deref<Target = Task>;
    type TaskRefMut: DerefMut<Target = Task>;

    /// Persists the provided `task`.
    /// If a `Task` with the same `task_id` already exists in the store, it is returned.
    async fn persist_task(&self, task: VotingTask) -> Result<Option<Task>, Error>;

    /// Returns a reference to the `Task` referenced by the provided `task_id`.
    /// Returns an error if the provided `task_id`
    /// does not reference an existing `Task`
    async fn get_task(&self, task_id: &Uuid) -> Result<Self::TaskRef, Error>;

    /// Returns a mutable reference to the `Task` referenced by the provided `task_id`.
    /// Returns an error if the provided `task_id`
    /// does not reference an existing `Task`
    async fn get_task_mut(&self, task_id: &Uuid) -> Result<Self::TaskRefMut, Error>;
}

#[cfg(test)]
use mockall::mock;

#[cfg(test)]
mock! {
    pub TaskStore {
        pub fn persist_task(&self, task: VotingTask) ->
            Result<Option<Task>, Error>;
        pub fn get_task(&self, task_id: &Uuid) ->
            Result<<Self as TaskStore>::TaskRef, Error>;
        pub fn get_task_mut(&self, task_id: &Uuid) ->
            Result<<Self as TaskStore>::TaskRefMut, Error>;
    }
}

// TODO: Mock the async behavior as well.
#[cfg(test)]
#[async_trait]
impl TaskStore for MockTaskStore {
    type TaskRef = Box<Task>;
    type TaskRefMut = Box<Task>;

    async fn persist_task(&self, task: VotingTask) -> Result<Option<Task>, Error> {
        self.persist_task(task)
    }
    async fn get_task(&self, task_id: &Uuid) -> Result<Self::TaskRef, Error> {
        self.get_task(task_id)
    }
    async fn get_task_mut(&self, task_id: &Uuid) -> Result<Self::TaskRefMut, Error> {
        self.get_task_mut(task_id)
    }
}
