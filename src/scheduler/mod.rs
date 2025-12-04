pub mod build_vote_whitelist;
mod check_vote_meta_tx;
mod check_vote_tx;

use color_eyre::{Result, eyre::eyre};
use tokio_cron_scheduler::JobScheduler;

use crate::AppView;

pub async fn init_task_scheduler(app: &AppView) -> Result<()> {
    let mut sched = JobScheduler::new().await?;

    let job = build_vote_whitelist::job(&sched, app, "0 0 0 * * *").await?;
    sched.add(job).await?;

    let job = check_vote_meta_tx::job(&sched, app, "1/10 * * * * *").await?;
    sched.add(job).await?;

    let job = check_vote_tx::job(&sched, app, "1/15 * * * * *").await?;
    sched.add(job).await?;

    sched.set_shutdown_handler(Box::new(|| {
        Box::pin(async move {
            error!("scheduler shut down");
        })
    }));

    sched.start().await.map_err(|e| eyre!(e))
}
