//! Your review of a game: read, write, change and delete it in the app, like
//! the "Write a review" box on a Steam store page.
//!
//! Reading and changing go through Steam's UserReviews service; a first
//! review is posted the way the store page posts it (there's no service call
//! for that), and deleting the way your profile's review page does.

use model::{AppId, MyReview, SteamId};
use net::Connection;
use proto::webui::{
    CUserReviewsGetIndividualRecommendationsRequest, CUserReviewsGetIndividualRecommendationsRequestRecommendationRequest, CUserReviewsGetIndividualRecommendationsResponse,
    CUserReviewsUpdateRequest, CUserReviewsUpdateResponse,
};

/// Your review of `app`, if you wrote one.
pub async fn mine(conn: &Connection, me: SteamId, app: AppId) -> Result<Option<MyReview>, String> {
    let req = CUserReviewsGetIndividualRecommendationsRequest {
        requests: vec![CUserReviewsGetIndividualRecommendationsRequestRecommendationRequest { steamid: Some(me), appid: Some(app) }],
    };
    let r: CUserReviewsGetIndividualRecommendationsResponse = conn.call("UserReviews.GetIndividualRecommendations#1", &req).await.map_err(|e| e.to_string())?;
    Ok(r.recommendations.into_iter().find(|d| d.appid() == app && d.recommendationid() != 0).map(|d| MyReview {
        id: d.recommendationid(),
        text: d.review().to_string(),
        voted_up: d.voted_up(),
        public: d.is_public(),
        comments_disabled: d.comments_disabled(),
        received_compensation: d.received_compensation(),
        language: d.language().to_string(),
        updated: d.time_updated().max(d.time_created()) as u64,
        votes_up: d.votes_up(),
        votes_funny: d.votes_funny(),
        comments: d.comment_count(),
    }))
}

/// Post a first review, or change the one you have (`review.id` set).
pub async fn save(conn: &Connection, web: &crate::market::Web, app: AppId, review: &MyReview) -> Result<(), String> {
    if review.text.trim().is_empty() {
        return Err("Write something about the game first.".into());
    }
    if review.id != 0 {
        let req = CUserReviewsUpdateRequest {
            recommendationid: Some(review.id),
            review_text: Some(review.text.clone()),
            voted_up: Some(review.voted_up),
            is_public: Some(review.public),
            language: Some(review.language.clone()),
            received_compensation: Some(review.received_compensation),
            comments_disabled: Some(review.comments_disabled),
            ..Default::default()
        };
        conn.call::<_, CUserReviewsUpdateResponse>("UserReviews.Update#1", &req).await.map_err(|e| e.to_string())?;
        return Ok(());
    }
    let flag = |b: bool| if b { "1" } else { "0" }.to_string();
    let r = web
        .http
        .post("https://store.steampowered.com/friends/recommendgame")
        .form(&[
            ("appid", app.to_string()),
            ("steamworksappid", app.to_string()),
            ("comment", review.text.clone()),
            ("rated_up", review.voted_up.to_string()),
            ("is_public", review.public.to_string()),
            ("language", review.language.clone()),
            ("received_compensation", flag(review.received_compensation)),
            ("disable_comments", flag(review.comments_disabled)),
            ("sessionid", web.session_id.clone()),
        ])
        .send()
        .await
        .map_err(|e| e.to_string())?;
    let v: serde_json::Value = r.json().await.map_err(|e| e.to_string())?;
    if v.get("success").and_then(|s| s.as_bool().or_else(|| s.as_i64().map(|n| n == 1))) == Some(true) {
        Ok(())
    } else {
        Err(v.get("strError").and_then(|e| e.as_str()).filter(|e| !e.is_empty()).unwrap_or("Steam didn't take the review. Try again in a moment.").to_string())
    }
}

/// Delete your review of `app`.
pub async fn delete(web: &crate::market::Web, app: AppId) -> Result<(), String> {
    let r = web
        .http
        .post(format!("https://steamcommunity.com/profiles/{}/recommended/", web.me))
        .form(&[("action", "delete".to_string()), ("sessionid", web.session_id.clone()), ("appid", app.to_string())])
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if r.status().is_success() { Ok(()) } else { Err(format!("Steam answered {}", r.status())) }
}
