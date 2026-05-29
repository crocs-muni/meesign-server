use crate::error::Error;
use crate::tasks::{Task, VotingTask};
use async_trait::async_trait;
use std::future::Future;
use std::ops::{Deref, DerefMut};
use std::pin::Pin;
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
    pub TaskStore<Ref, RefMut>
    where
        Ref: Deref<Target = Task> + Send,
        RefMut: DerefMut<Target = Task> + Send,
    {
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
// #[async_trait]
impl<Ref, RefMut> TaskStore for MockTaskStore<Ref, RefMut>
where
    Ref: Deref<Target = Task> + Send,
    RefMut: DerefMut<Target = Task> + Send,
{
    type TaskRef = Ref;
    type TaskRefMut = RefMut;

    fn persist_task<'life0, 'async_trait>(
        &'life0 self,
        task: VotingTask,
    ) -> Pin<Box<dyn Future<Output = Result<Option<Task>, Error>> + Send + 'async_trait>>
    where
        'life0: 'async_trait,
        Self: 'async_trait,
    {
        let res = self.persist_task(task);
        Box::pin(async move { res })
    }
    fn get_task<'life0, 'life1, 'async_trait>(
        &'life0 self,
        task_id: &'life1 Uuid,
    ) -> Pin<Box<dyn Future<Output = Result<Self::TaskRef, Error>> + Send + 'async_trait>>
    where
        'life0: 'async_trait,
        'life1: 'async_trait,
        Self: 'async_trait,
    {
        let res = self.get_task(task_id);
        Box::pin(async move { res })
    }
    fn get_task_mut<'life0, 'life1, 'async_trait>(
        &'life0 self,
        task_id: &'life1 Uuid,
    ) -> Pin<Box<dyn Future<Output = Result<Self::TaskRefMut, Error>> + Send + 'async_trait>>
    where
        'life0: 'async_trait,
        'life1: 'async_trait,
        Self: 'async_trait,
    {
        let res = self.get_task_mut(task_id);
        Box::pin(async move { res })
    }
}
