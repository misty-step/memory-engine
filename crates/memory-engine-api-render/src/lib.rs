#![cfg_attr(not(test), deny(clippy::expect_used, clippy::unwrap_used))]

//! Server-rendered HTML for the Memory Engine HTTP API.

mod render;

#[cfg(test)]
mod design_preview;

pub const LEDGER_CSS: &str = include_str!("../assets/ledger.css");

pub use render::{
    render_account_page, render_action_result_html, render_analytics_page, render_app_shell,
    render_auth_recovery, render_content_feedback_recovery_html,
    render_content_feedback_result_html, render_edit_review_html, render_login_requested,
    render_return_notification_confirmation, render_return_notification_disabled,
    render_submit_action_result_html, render_submit_recovery, render_waitlist_joined,
    AnalyticsConceptFilter, AnalyticsConceptSort, AnalyticsViewOptions, ContentFeedbackRecovery,
};
