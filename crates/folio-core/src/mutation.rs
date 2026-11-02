use std::future::Future;

use crate::models::UserId;

tokio::task_local! {
    static ACTOR: Option<UserId>;
}

/// Runs a mutation as an authenticated user, or as the system for `None`.
/// The actor is restored on completion or cancellation and is not inherited by spawned tasks.
pub async fn with_actor<T>(actor: Option<UserId>, future: impl Future<Output = T>) -> T {
    ACTOR.scope(actor, future).await
}

/// Returns the scoped authenticated user, or `None` for system work and unscoped calls.
/// This context records attribution; it does not authorize access.
pub fn current_actor() -> Option<UserId> {
    ACTOR.try_with(Clone::clone).ok().flatten()
}

#[cfg(test)]
mod tests {
    use super::{current_actor, with_actor};
    use crate::models::UserId;

    #[tokio::test]
    async fn actor_is_scoped_and_restored_after_nested_system_work() {
        let user = UserId::from("alice");
        assert_eq!(current_actor(), None);
        with_actor(Some(user.clone()), async {
            assert_eq!(current_actor(), Some(user.clone()));
            with_actor(None, async { assert_eq!(current_actor(), None) }).await;
            assert_eq!(current_actor(), Some(user));
        })
        .await;
        assert_eq!(current_actor(), None);
    }

    #[tokio::test]
    async fn concurrent_requests_keep_independent_actors_across_yields() {
        let alice = with_actor(Some(UserId::from("alice")), async {
            tokio::task::yield_now().await;
            current_actor()
        });
        let bob = with_actor(Some(UserId::from("bob")), async {
            tokio::task::yield_now().await;
            current_actor()
        });
        let (alice, bob) = tokio::join!(alice, bob);
        assert_eq!(alice, Some(UserId::from("alice")));
        assert_eq!(bob, Some(UserId::from("bob")));
        assert_eq!(current_actor(), None);
    }

    #[tokio::test]
    async fn cancelling_a_mutation_restores_the_enclosing_actor() {
        use std::future::Future;
        use std::task::Poll;

        with_actor(Some(UserId::from("outer")), async {
            let mut mutation = Box::pin(with_actor(Some(UserId::from("cancelled")), async {
                assert_eq!(current_actor(), Some(UserId::from("cancelled")));
                std::future::pending::<()>().await;
            }));
            std::future::poll_fn(|cx| {
                assert!(mutation.as_mut().poll(cx).is_pending());
                Poll::Ready(())
            })
            .await;
            drop(mutation);
            assert_eq!(current_actor(), Some(UserId::from("outer")));
        })
        .await;
        assert_eq!(current_actor(), None);
    }

    #[tokio::test]
    async fn detached_tasks_do_not_inherit_the_request_actor() {
        with_actor(Some(UserId::from("alice")), async {
            let actor = tokio::spawn(async { current_actor() }).await.unwrap();
            assert_eq!(actor, None);
            assert_eq!(current_actor(), Some(UserId::from("alice")));
        })
        .await;
    }
}
