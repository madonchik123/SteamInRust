//! Samples shaped like Steam's pages, with made-up people, ids and text.

use super::*;

const ACTIVITY: &str = r##"<div class="blotter_day" id="blotter_day_1700000000">
	<div class="blotter_day_header">
		<div class="blotter_day_header_date">12 Sep</div>
		<div class="blotter_day_header_divider"></div>
	</div>
	<div class="blotter_block">
<div class="blotter_daily_rollup">
	<div class="blotter_daily_rollup_line">
		<div class="blotter_rollup_avatar">
			<div class="blotter_small_friend_block_container">
				<a href="https://steamcommunity.com/id/sample_one">
					<div class="blotter_playerAvatarSmall offline" >
						<img src="https://avatars.akamai.steamstatic.com/aaaa.jpg" data-miniprofile="1001">
					</div>
				</a>
			</div>
		</div>
		<span>
		<a href="https://steamcommunity.com/id/sample_one" data-miniprofile="1001">Sample One</a> and <a href="https://steamcommunity.com/id/sample_two" data-miniprofile="1002">Sample Two</a> are now friends.		</span>
		<div style="clear: inline-start;"></div>
	</div>
	<div class="blotter_daily_rollup_line">
		<div class="blotter_rollup_avatar">
			<div class="blotter_small_friend_block_container">
				<a href="https://steamcommunity.com/profiles/76561197960266730">
					<div class="blotter_playerAvatarSmall online" ><img src="https://avatars.akamai.steamstatic.com/bbbb.jpg" data-miniprofile="1002"></div>
				</a>
			</div>
		</div>
		<span>
		<a href="https://steamcommunity.com/profiles/76561197960266730" data-miniprofile="1002">Sample Two</a> has added <a href="https://steamcommunity.com/app/620">Portal 2</a> to their wishlist.		</span>
	</div>
	<div class="blotter_daily_rollup_line">
		<div class="blotter_rollup_avatar"><img src="https://avatars.akamai.steamstatic.com/bbbb.jpg" data-miniprofile="1002"></div>
		<span>
		<a href="https://steamcommunity.com/profiles/76561197960266730" data-miniprofile="1002">Sample Two</a> achieved <a href="https://steamcommunity.com/app/620">Portal 2</a>:				</span>
		<img src="https://shared.akamai.steamstatic.com/community_assets/images/apps/620/ach.jpg" width="32" height="32"
title="Wake Up Call
Survive the manual override.">
		<div style="clear: inline-start;"></div>
	</div>
</div>
	</div>
	<div class="blotter_block">
<div class="blotter_workshopitempublished" data-appid="440" data-snr="x">
	<div class="blotter_author_block">
		<div class="blotter_avatar_holder">
			<a href="https://steamcommunity.com/id/sample_one">
				<div class="playerAvatar offline" >
					<div class="profile_avatar_frame"><picture><img src="https://shared.akamai.steamstatic.com/community_assets/images/items/1/frame.png"></picture></div>
					<picture><img srcset="https://avatars.akamai.steamstatic.com/aaaa_medium.jpg" data-miniprofile="1001"></picture>
				</div>
			</a>
		</div>
		<div>
			<a href="https://steamcommunity.com/id/sample_one" data-miniprofile="1001">Sample One</a>		</div>
		<div>
			published artwork for <a href="https://steamcommunity.com/app/440">Team Fortress 2</a>		</div>
	</div>
	<div class="blotter_workshopitem artwork">
		<div class="blotter_workshopitem_preview artwork">
			<a href="https://steamcommunity.com/sharedfiles/filedetails/?id=555" class="modalContentLink ugc" data-appid="440" data-publishedfileid="555">
				<img src="https://images.steamusercontent.com/ugc/1/ABC/?imw=320&amp;imh=181" />
			</a>
		</div>
		<div class="blotter_workshopitem_metadata">
			<div class="blotter_workshopitem_title">
				<a href="https://steamcommunity.com/sharedfiles/filedetails/?id=555">A drawing</a>
			</div>
			<div class="blotter_workshopitem_description">My first one &gt;w&lt;</div>
		</div>
	</div>
	<div>
		<div class="blotter_control_container">
			<a onclick="return VoteUp(555);" class="btn_grey_grey"><span>Rate up</span></a>
			<span class="blotter_voters_names" id="blotter_publishedfile_voters555">
				8 people rated this up			</span>
		</div>
	</div>
	<div class="blotter_comment_thread">
		<div class="blotter_viewallcomments_container" style="display: none">
			<a href="https://steamcommunity.com/sharedfiles/filedetails/?id=555"><span><i class="ico16 comment"></i> View all 3 comments</span></a>
		</div>
	</div>
</div>
	</div>
	<div class="blotter_block">
<div class="blotter_recommendation" data-appid="620" data-snr="x">
	<div class="blotter_author_block">
		<div class="blotter_avatar_holder">
			<a href="https://steamcommunity.com/profiles/76561197960266730"><div class="playerAvatar offline"><picture><img srcset="https://avatars.akamai.steamstatic.com/bbbb_medium.jpg" data-miniprofile="1002"></picture></div></a>
		</div>
		<div>
			<a href="https://steamcommunity.com/profiles/76561197960266730" data-miniprofile="1002">Sample Two</a>		</div>
		reviewed <a href="https://steamcommunity.com/app/620">Portal 2</a>	</div>
	<div class="blotter_recommendation_content">
		<div class="apphub_CardContentMain">
			<div class="apphub_UserReviewCardContent">
				<div class="found_helpful">
					12 people found this review helpful<br>3 people found this review funny				</div>
				<div class="vote_header">
					<a href="https://steamcommunity.com/profiles/76561197960266730/recommended/620/"><img src="https://shared.akamai.steamstatic.com/capsule.jpg" height="44"></a>
					<div class="thumb"><img src="https://community.akamai.steamstatic.com/public/shared/images/userreviews/icon_thumbsUp.png" width="44" height="44"></div>
					<div class="title"><a href="https://steamcommunity.com/profiles/76561197960266730/recommended/620/">Recommended</a></div>
					<div class="hours">3 hrs last two weeks / 30.5 hrs on record</div>
				</div>
				<div class="apphub_CardTextContent responsive_body_text">best game ever<br>really</div>
			</div>
		</div>
	</div>
</div>
	</div>
	<div class="blotter_block">
<div class="blotter_gamepurchase">
	<div class="blotter_author_block">
		<div class="blotter_avatar_holder"><a href="https://steamcommunity.com/id/sample_one"><div class="playerAvatar"><picture><img srcset="https://avatars.akamai.steamstatic.com/aaaa_medium.jpg" data-miniprofile="1001"></picture></div></a></div>
		<div><a href="https://steamcommunity.com/id/sample_one" data-miniprofile="1001">Sample One</a>		</div>
		<div>now owns <a href="https://store.steampowered.com/app/400/Portal">Portal</a></div>
	</div>
	<div class="blotter_gamepurchase_content">
		<a href="https://store.steampowered.com/app/400/Portal" class="blotter_gamepurchase_logo single_item" data-appid="400"><img src="https://shared.akamai.steamstatic.com/apps/400/capsule_231x87.jpg" /></a>
		<span class="blotter_gamepurchase_text">A puzzle game with portals.</span>
	</div>
</div>
	</div>
	<div class="blotter_block">
<div class="blotter_userstatus" id="group_announcement1">
	<div class="blotter_group_announcement_header_ogg">
		<div class="blotter_rollup_avatar blotter_group_container">
			<div class="gameLogo"><a href="https://steamcommunity.com/app/570"><img src="https://shared.akamai.steamstatic.com/apps/570/capsule_184x69.jpg"/></a></div>
		</div>
		<div class="blotter_group_announcement_header_text">
			<a href="https://steamcommunity.com/app/570">Sample Game</a><br>Announcement		</div>
	</div>
	<div class="blotter_group_announcement_content responsive_body_text">
		<div class="blotter_group_announcement_headline">
			<a href="https://steamcommunity.com/games/570/announcements/detail/1">Big update</a>
		</div>
		<div class="group_announcement_auto_collapse" style="max-height: 500px" >
			<div class="bb_h2">What's new</div><p class="bb_paragraph">New maps.</p><img src="https://clan.akamai.steamstatic.com/images/1/banner.png">
		</div>
	</div>
</div>
	</div>
	<div class="blotter_block">
<div class="blotter_userstatus" id="userstatus_1_2">
	<div class="blotter_author_block">
		<div class="blotter_avatar_holder"><a href="https://steamcommunity.com/id/sample_one"><div class="playerAvatar"><picture><img srcset="https://avatars.akamai.steamstatic.com/aaaa_medium.jpg" data-miniprofile="1001"></picture></div></a></div>
		<div class="blotter_userstatus_game"><a class="blotter_userstats_game" href="https://steamcommunity.com/app/440"><img src="https://shared.akamai.steamstatic.com/apps/440/capsule_184x69.jpg"></a></div>
		<div><a href="https://steamcommunity.com/id/sample_one">Sample One</a></div>
		<div>posted a status about <a href="https://steamcommunity.com/app/440">Team Fortress 2</a></div>
	</div>
	<div class="blotter_userstatus_content responsive_body_text">meow<img src="https://community.akamai.steamstatic.com/economy/emoticon/x" alt=":x:" class="emoticon"></div>
	<a href="https://steamcommunity.com/id/sample_one/status/1" class="btn_grey_grey"></a>
</div>
	</div>
</div>"##;

#[test]
fn reads_friends_activity() {
    let body = serde_json::json!({
        "success": true,
        "timestart": 1700000000u64,
        "blotter_html": ACTIVITY,
        "next_request": "https://steamcommunity.com/profiles/76561197960266729/ajaxgetusernews/?start=1699913600",
    })
    .to_string();
    let feed = parse_activity(&body).unwrap();
    assert_eq!(feed.next, 1699913600);
    let kinds: Vec<PostKind> = feed.posts.iter().map(|p| p.kind).collect();
    assert_eq!(kinds, [PostKind::Status, PostKind::Status, PostKind::Achievement, PostKind::Artwork, PostKind::Review, PostKind::Purchase, PostKind::Announcement, PostKind::Status]);
    assert!(feed.posts.iter().all(|p| p.when == "12 Sep"));

    let friends = &feed.posts[0];
    assert_eq!(friends.author, BASE_ID + 1001);
    assert_eq!(friends.author_name, "Sample One");
    assert_eq!(friends.action, "Sample One and Sample Two are now friends.");
    assert_eq!(friends.avatar.as_deref(), Some("https://avatars.akamai.steamstatic.com/aaaa.jpg"));
    let wish = &feed.posts[1];
    assert_eq!(wish.app_id, 620);
    assert_eq!(wish.author, BASE_ID + 1002);

    let ach = &feed.posts[2];
    assert_eq!(ach.action, "Sample Two achieved Portal 2 :");
    assert_eq!(ach.images, ["https://shared.akamai.steamstatic.com/community_assets/images/apps/620/ach.jpg"]);
    assert_eq!(ach.text, "Wake Up Call\nSurvive the manual override.");

    let art = &feed.posts[3];
    assert_eq!((art.author, art.author_name.as_str(), art.app_id, art.file_id), (BASE_ID + 1001, "Sample One", 440, 555));
    assert_eq!(art.action, "published artwork for Team Fortress 2");
    assert_eq!(art.images, ["https://images.steamusercontent.com/ugc/1/ABC/?imw=320&imh=181"]);
    assert_eq!((art.title.as_str(), art.text.as_str()), ("A drawing", "My first one >w<"));
    assert_eq!((art.votes_up, art.comments), (8, 3));
    assert_eq!(art.avatar.as_deref(), Some("https://avatars.akamai.steamstatic.com/aaaa_medium.jpg"));
    assert_eq!(art.url, "https://steamcommunity.com/sharedfiles/filedetails/?id=555");

    let review = &feed.posts[4];
    assert_eq!(review.recommended, Some(true));
    assert_eq!(review.hours, "3 hrs last two weeks / 30.5 hrs on record");
    assert_eq!(review.text, "best game ever\nreally");
    assert_eq!((review.votes_up, review.funny), (12, 3));
    assert_eq!(review.url, "https://steamcommunity.com/profiles/76561197960266730/recommended/620/");
    assert_eq!(review.action, "reviewed Portal 2");

    let bought = &feed.posts[5];
    assert_eq!((bought.app_id, bought.text.as_str()), (400, "A puzzle game with portals."));
    assert_eq!(bought.action, "now owns Portal");

    let news = &feed.posts[6];
    assert_eq!((news.app_id, news.game.as_str(), news.title.as_str()), (570, "Sample Game", "Big update"));
    assert_eq!(news.url, "https://steamcommunity.com/games/570/announcements/detail/1");
    assert!(news.text.starts_with("What's new\nNew maps."), "{:?}", news.text);
    assert_eq!(news.images, ["https://clan.akamai.steamstatic.com/images/1/banner.png"]);

    let status = &feed.posts[7];
    assert_eq!((status.author, status.app_id), (BASE_ID + 1001, 440));
    assert_eq!(status.author_name, "Sample One");
    assert_eq!(status.action, "posted a status about Team Fortress 2");
    assert_eq!(status.text, "meow");
    assert!(status.images.is_empty());
}

#[test]
fn rejects_a_failed_answer() {
    assert!(parse_activity(r#"{"success":false}"#).is_err());
    assert!(parse_activity("<html>sign in</html>").is_err());
    let empty = parse_activity(r#"{"success":true,"blotter_html":""}"#).unwrap();
    assert!(empty.posts.is_empty() && empty.next == 0);
}

const EXPLORE: &str = r##"<div id="page1">
	<div data-panel="{}" role="button" class="apphub_Card modalContentLink interactable" style="display: none" data-modal-content-url="https://steamcommunity.com/profiles/76561197960266731/recommended/620/" data-modal-content-sizetofit="false">
	<div class="apphub_CardContentMain">
		<div class="apphub_UserReviewCardContent">
			<div class="found_helpful">
				1,579 people found this review helpful<br>6 people found this review funny				<div class="review_award_aggregated tooltip" data-tooltip-html="&lt;div&gt;"><img src="https://community.akamai.steamstatic.com/award_icon_blue.svg">92</div>
			</div>
			<div class="vote_header">
				<div class="reviewInfo">
					<div class="thumb"><img src="https://community.akamai.steamstatic.com/public/shared/images/userreviews/icon_thumbsDown.png?v=1" width="44" height="44"></div>
					<div class="title">Not Recommended</div>
					<div class="hours">168.7 hrs on record</div>
				</div>
			</div>
			<div class="apphub_CardTextContent">
				<div class="date_posted">Posted: November 12</div>
				too hard<br>still playing			</div>
		</div>
	</div>
	<div class="apphub_CardContentAuthorBlock tall">
		<div class="apphub_CardContentAppName"><a href="https://steamcommunity.com/app/620">Portal 2</a></div>
		<div class="apphub_UserReviewCardStats"><div class="apphub_CardCommentCount alignUserReviews">8</div></div>
	</div>
</div>
	<div data-panel="{}" role="button" class="apphub_Card modalContentLink interactable ugc"
style="display: none" data-modal-content-url="https://steamcommunity.com/sharedfiles/filedetails/?id=777"	 id="apphub_Card_777"
data-appid="440" data-publishedfileid="777">
	<div class="apphub_CardContentClickable">
		<div class="apphub_CardContentTypeContainer"><div class="apphub_CardContentType">Team Fortress 2: Screenshot</div></div>
		<div class="apphub_CardContentPreviewImageBorder">
			<div class="apphub_CardContentMain"
>
				<img class="apphub_CardContentPreviewImage" src="https://images.steamusercontent.com/ugc/2/DEF/?imw=1024&amp;ima=fit" width="1024" height="576" >
			</div>
		</div>
	</div>
	<div class="apphub_CardMetaData">
		<div class="apphub_CardControls">
			<div class="apphub_CardRightData">
				<div class="apphub_CardRating rateUp withRateButtons">
				336					</div>
				<div class="apphub_CardCommentCount">19</div>
			</div>
		</div>
		<div class="apphub_CardContentTitle ellipsis">sunset&nbsp;</div>
	</div>
</div>
	<div data-panel="{}" role="button" class="apphub_Card interactable modalContentLink" style="display: none" data-modal-content-url="https://steamcommunity.com/sharedfiles/filedetails/?id=888" id="apphub_Card_888">
	<div class="apphub_CardContentClickable">
		<div class="apphub_CardContentPreviewImageBorder">
			<div class="apphub_CardContentMain">
				<div class="apphub_CardContentType guide">Sample Game: Guide</div>
				<div class="apphub_CardContentGuideTitle">
					<img class="apphub_CardContentGuideImage" src="https://images.steamusercontent.com/ugc/3/GHI/?imw=128">
					All endings				</div>
				<div class="apphub_CardContentGuideDesc">How to get every ending.</div>
			</div>
		</div>
	</div>
	<div class="apphub_CardMetaData"><div class="apphub_CardControls"><div class="apphub_CardRating fourStar starRating">53 ratings</div></div></div>
	<div class="apphub_CardContentAuthorBlock tall">
		<div class="apphub_CardContentAppName ellipsis" ><a href="https://steamcommunity.com/app/2638080">Sample Game</a></div>
	</div>
</div>
	<div data-panel="{}" role="button" class="Broadcast_Card apphub_Card interactable" style="display: none">
	<a href="https://steamcommunity.com/broadcast/watch/76561197960266732">
		<div class="apphub_CardContentClickable">
			<div class="apphub_CardContentTypeContainer"><div class="apphub_CardContentType">Sample Racer: Broadcast</div></div>
			<div class="apphub_CardContentPreviewImageBorder"><div class="apphub_CardContentMain">
				<img class="apphub_CardContentPlayVideoIcon" src="https://community.akamai.steamstatic.com/play_icon80.png">
				<img class="apphub_CardContentPreviewImage" src="https://steambroadcast.akamaized.net/broadcast/1/thumbnail/" width="313" height="176">
			</div></div>
		</div>
		<div class="apphub_CardMetaData">
			<div class="apphub_CardContentViewers ellipsis">3,841 viewers&nbsp;</div>
			<div class="apphub_CardContentTitle ellipsis">Sample Racer</div>
		</div>
		<div class="apphub_CardContentAuthorBlock tall">
			<div class="apphub_friend_block_container"><a href="https://steamcommunity.com/profiles/76561197960266732/">
				<div class="apphub_friend_block" data-miniprofile="1004">
					<div class="appHubIconHolder offline"><img src="https://avatars.akamai.steamstatic.com/dddd.jpg"></div>
					<div class="apphub_CardContentAuthorName offline ellipsis"><a href="https://steamcommunity.com/profiles/76561197960266732/">Sample Streamer</a></div>
				</div>
			</a></div>
		</div>
	</a>
</div>
</div>"##;

#[test]
fn reads_community_cards() {
    let posts = parse_explore(EXPLORE);
    let kinds: Vec<PostKind> = posts.iter().map(|p| p.kind).collect();
    assert_eq!(kinds, [PostKind::Review, PostKind::Screenshot, PostKind::Guide, PostKind::Broadcast]);

    let review = &posts[0];
    assert_eq!((review.app_id, review.game.as_str()), (620, "Portal 2"));
    assert_eq!(review.recommended, Some(false));
    assert_eq!(review.hours, "168.7 hrs on record");
    assert_eq!(review.when, "Posted: November 12");
    assert_eq!(review.text, "too hard\nstill playing");
    assert_eq!((review.votes_up, review.funny, review.comments), (1579, 6, 8));
    assert_eq!(review.author, 76561197960266731);

    let shot = &posts[1];
    assert_eq!((shot.app_id, shot.file_id, shot.game.as_str(), shot.title.as_str()), (440, 777, "Team Fortress 2", "sunset"));
    assert_eq!(shot.images, ["https://images.steamusercontent.com/ugc/2/DEF/?imw=1024&ima=fit"]);
    assert_eq!((shot.votes_up, shot.comments), (336, 19));

    let guide = &posts[2];
    assert_eq!((guide.title.as_str(), guide.text.as_str(), guide.app_id), ("All endings", "How to get every ending.", 2638080));
    assert_eq!(guide.votes_up, 0);

    let live = &posts[3];
    assert!(live.live);
    assert_eq!((live.views, live.author, live.author_name.as_str()), (3841, BASE_ID + 1004, "Sample Streamer"));
    assert_eq!(live.url, "https://steamcommunity.com/broadcast/watch/76561197960266732");
    assert_eq!(live.game, "Sample Racer");
    assert_eq!(live.avatar.as_deref(), Some("https://avatars.akamai.steamstatic.com/dddd.jpg"));
}
