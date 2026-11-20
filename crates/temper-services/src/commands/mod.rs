//! The server's write-command vocabulary: the commands `DbBackend` runs, what each returns
//! (`CommandOutput`), and the events a command reports (`DomainEvent`).
//!
//! It is server-only. A door builds a wire request (`temper_workflow::shaping` shapes the
//! resource writes) and the API handler turns that request into one of these commands, so no
//! published crate carries this vocabulary.

mod command;
mod events;
mod inputs;
mod output;

pub use command::{
    AdvanceStewardWatermark, AnnotateResource, AssertRelationship, AuditorDispatchTick,
    CloseInvocation, CommitDataArtifact, CompleteAuditorJob, CreateCognitiveMap, CreateResource,
    DeleteResource, FoldRelationship, GoalPatch, ListResources, MaterializeOnThreshold, MoveSpec,
    OpenInvocation, ReblockResources, ReconcileCognitiveMap, RecordCitationAudit, RetractFacet,
    RetypeRelationship, ReweightRelationship, SearchResources, SetFacet, ShowResource,
    StewardDispatchTick, UpdateResource,
};
pub use events::{DomainEvent, PushDeferReason};
pub use inputs::{BodyUpdate, ListFilter, SearchQuery};
pub use output::CommandOutput;
