pub mod build_vote_whitelist;

use color_eyre::{Result, eyre::eyre};
use tokio_cron_scheduler::JobScheduler;

use crate::AppView;

pub async fn init_task_scheduler(app: &AppView) -> Result<()> {
    let mut sched = JobScheduler::new().await?;

    let job = build_vote_whitelist::build_vote_whitelist_job(&sched, app).await?;

    sched.add(job).await?;

    sched.set_shutdown_handler(Box::new(|| {
        Box::pin(async move {
            error!("scheduler shut down");
        })
    }));

    sched.start().await.map_err(|e| eyre!(e))
}
