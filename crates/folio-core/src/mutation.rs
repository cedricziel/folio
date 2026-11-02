use std::future::Future;

use uuid::Uuid;

use crate::models::UserId;

/// Trusted correlation and causation for events emitted by a mutation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MutationLineage {
    pub correlation_id: Uuid,
    pub causation_id: Option<Uuid>,
}

tokio::task_local! {
    static ACTOR: Option<UserId>;
    static LINEAGE: Option<MutationLineage>;
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

/// Runs mutations with trusted event lineage, or clears lineage for `None`.
/// The previous lineage is restored on completion or cancellation; spawned tasks do not inherit it.
pub async fn with_lineage<T>(
    lineage: Option<MutationLineage>,
    future: impl Future<Output = T>,
) -> T {
    LINEAGE.scope(lineage, future).await
}

/// Returns the scoped server lineage, independently of actor attribution.
pub fn current_lineage() -> Option<MutationLineage> {
    LINEAGE.try_with(|lineage| *lineage).ok().flatten()
}

#[cfg(test)]
mod tests {
    use super::{current_actor, current_lineage, with_actor, with_lineage, MutationLineage};
    use crate::models::UserId;
    use uuid::Uuid;

    fn lineage(id: u128) -> MutationLineage {
        MutationLineage {
            correlation_id: Uuid::from_u128(id),
            causation_id: Some(Uuid::from_u128(id + 1)),
        }
    }

    #[tokio::test]
    async fn lineage_is_restored_after_nested_changes_and_explicit_clearing() {
        let outer = lineage(1);
        let inner = lineage(3);
        assert_eq!(current_lineage(), None);
        with_lineage(Some(outer), async {
            assert_eq!(current_lineage(), Some(outer));
            with_lineage(Some(inner), async {
                assert_eq!(current_lineage(), Some(inner));
            })
            .await;
            assert_eq!(current_lineage(), Some(outer));
            with_lineage(None, async { assert_eq!(current_lineage(), None) }).await;
            assert_eq!(current_lineage(), Some(outer));
        })
        .await;
        assert_eq!(current_lineage(), None);
    }

    #[tokio::test]
    async fn concurrent_mutations_keep_independent_lineage_across_yields() {
        let first = with_lineage(Some(lineage(1)), async {
            tokio::task::yield_now().await;
            current_lineage()
        });
        let second = with_lineage(Some(lineage(3)), async {
            tokio::task::yield_now().await;
            current_lineage()
        });
        let (first, second) = tokio::join!(first, second);
        assert_eq!(first, Some(lineage(1)));
        assert_eq!(second, Some(lineage(3)));
        assert_eq!(current_lineage(), None);
    }

    #[tokio::test]
    async fn cancelling_a_mutation_restores_the_enclosing_lineage() {
        use std::future::Future;
        use std::task::Poll;

        let outer = lineage(1);
        let cancelled = lineage(3);
        with_lineage(Some(outer), async {
            let mut mutation = Box::pin(with_lineage(Some(cancelled), async {
                assert_eq!(current_lineage(), Some(cancelled));
                std::future::pending::<()>().await;
            }));
            std::future::poll_fn(|cx| {
                assert!(mutation.as_mut().poll(cx).is_pending());
                Poll::Ready(())
            })
            .await;
            drop(mutation);
            assert_eq!(current_lineage(), Some(outer));
        })
        .await;
        assert_eq!(current_lineage(), None);
    }

    #[tokio::test]
    async fn detached_tasks_do_not_inherit_mutation_lineage() {
        with_lineage(Some(lineage(1)), async {
            let inherited = tokio::spawn(async { current_lineage() }).await.unwrap();
            assert_eq!(inherited, None);
            assert_eq!(current_lineage(), Some(lineage(1)));
        })
        .await;
    }

    #[tokio::test]
    async fn actor_and_lineage_scopes_change_independently() {
        let actor = UserId::from("alice");
        with_actor(
            Some(actor.clone()),
            with_lineage(Some(lineage(1)), async {
                with_actor(None, async {
                    assert_eq!(current_actor(), None);
                    assert_eq!(current_lineage(), Some(lineage(1)));
                })
                .await;
                with_lineage(None, async {
                    assert_eq!(current_actor(), Some(actor.clone()));
                    assert_eq!(current_lineage(), None);
                })
                .await;
                assert_eq!(current_actor(), Some(actor));
                assert_eq!(current_lineage(), Some(lineage(1)));
            }),
        )
        .await;
        assert_eq!(current_actor(), None);
        assert_eq!(current_lineage(), None);
    }

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
