pub mod build_vote_whitelist;
mod check_vote_finished;
mod check_vote_meta_tx;
mod check_vote_tx;

use color_eyre::{Result, eyre::eyre};
use tokio_cron_scheduler::JobScheduler;

use crate::AppView;

pub async fn init_task_scheduler(app: &AppView) -> Result<()> {
    let mut scheduler = JobScheduler::new().await?;

    let job = build_vote_whitelist::job(&scheduler, app, "0 0 0 * * *").await?;
    scheduler.add(job).await?;

    let job = check_vote_meta_tx::job(&scheduler, app, "1/10 * * * * *").await?;
    scheduler.add(job).await?;

    let job = check_vote_tx::job(&scheduler, app, "1/15 * * * * *").await?;
    scheduler.add(job).await?;

    let job = check_vote_finished::job(&scheduler, app, "0 * * * * *").await?;
    scheduler.add(job).await?;

    scheduler.set_shutdown_handler(Box::new(|| {
        Box::pin(async move {
            error!("scheduler shut down");
        })
    }));

    scheduler.start().await.map_err(|e| eyre!(e))
}
