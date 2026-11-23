// Lane C — async package intelligence store and worker.

mod store;
mod worker;

pub use store::IntelligenceStore;
#[allow(unused_imports)]
pub use worker::{
    generate_and_store, intelligence_auto_enabled, intelligence_enabled, schedule_for_block,
};
