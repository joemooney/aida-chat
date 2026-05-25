// trace:EPIC-29 | ai:claude
//
// Agile metrics charts. Hand-crafted SVG, no external chart library —
// matches the aida-web-react ethos. The visual IP and algorithmic IP
// are ported from `~/ai/aida/aida-web-react/src/lib/sprint-utils.ts`
// and `~/ai/aida/aida-web-react/src/components/{sprint/charts,dashboard}/*.tsx`.
//
// Layout:
//   * store.rs   — reads AIDA's distributed substrate (.aida-store/objects/**/*.yaml)
//                  into typed `Requirement`s. Pure-Rust, no aida CLI required.
//   * data.rs    — ports the algorithmic core of sprint-utils.ts:
//                  computeBurndownData / computeBurnupData /
//                  computeVelocityData / computeSprintProgress.
//   * svg.rs     — five V1 chart renderers: status (donut), burn-down
//                  (line vs ideal), burn-up (line + area + scope),
//                  velocity (bars + avg), feature progress (h-bars).

pub mod data;
pub mod store;
pub mod svg;

pub use data::{
    BurndownPoint, BurnupPoint, FeatureProgressRow, SprintProgress, SprintState, StatusCounts,
    VelocityPoint,
};
pub use store::{AidaStore, Relationship, Requirement, Sprint, StoreError};
pub use svg::{
    render_burndown_svg, render_burnup_svg, render_feature_progress_svg, render_status_svg,
    render_velocity_svg, SvgError,
};
