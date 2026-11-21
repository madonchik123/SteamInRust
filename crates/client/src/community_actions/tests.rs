//! Samples shaped like Steam's pages, with made-up names, ids and tokens.

use super::*;

const ME: SteamId = 76561197960265729;
const GROUP: SteamId = 103582791429521412;

/// A game forum as a signed-in visitor gets it.
const FORUM: &str = r##"<script>
InitializeForum( "General_1000", {"owner":"1000","type":"General","feature":"0","gidforum":"86","forum_display_name":"General Discussions","forum_url":"https:\/\/steamcommunity.com\/app\/620\/discussions\/0\/","is_public":1,"permissions":{"can_view":1,"can_post":1,"can_reply":1,"is_banned":0},"appid":620,"start":0,"total_count":221,"pagesize":15}, 'https://steamcommunity.com/forum/1000/General/' );
</script>
<form id="forum_General_1000_newtopic_form" method="POST" action="https://steamcommunity.com" onsubmit="return false;">
	<input type="hidden" name="sessionid" value="sample">
	<input type="hidden" name="appid" value="620">
	<input name="topic" type="text" placeholder="enter a title" class="forum_topic_input"><br>
	<textarea rows="3" cols="40" class="forumtopic_reply_textarea" name="text" placeholder="say something"></textarea>
	Post in: <select name="subforum" class="gray_bevel subforum_select_control">
		<option value="General_0" data-forumtype="General" data-gidfeature="0" selected>General Discussions</option>
		<option value="Workshop_18446744073709551615" data-forumtype="Workshop" data-gidfeature="18446744073709551615" >Workshop &amp; Mods</option>
		<option value="Trading_18446744073709551615" data-forumtype="Trading" data-gidfeature="18446744073709551615" >Trading</option>
	</select>
</form>"##;

/// A group forum: no game, a forum other than the first.
const GROUP_FORUM: &str = r##"<script>
InitializeForum( "General_2000_3", {"owner":"2000","type":"General","feature":"3","forum_display_name":"Feedback","forum_url":"https:\/\/steamcommunity.com\/groups\/samplegroup\/discussions\/3\/","permissions":{"can_view":1,"can_post":1,"can_reply":1,"is_banned":0},"start":0,"total_count":7,"pagesize":15}, 'https://steamcommunity.com/forum/2000/General/' );
</script>
<form id="forum_General_2000_3_newtopic_form" method="POST" onsubmit="return false;">
	<input type="hidden" name="sessionid" value="sample">
	<input name="topic" type="text"><textarea name="text"></textarea>
	<select name="subforum">
		<option value="General_3" selected>Feedback</option>
		<option value="General_1">General Discussions</option>
	</select>
</form>"##;

/// Signed out, Steam sends no form.
const SIGNED_OUT: &str = r##"InitializeForum( "General_1000", {"owner":"1000","type":"General","feature":"0","forum_url":"https:\/\/steamcommunity.com\/app\/620\/discussions\/0\/","permissions":{"can_view":1,"can_post":1,"can_reply":1,"is_banned":0},"appid":620}, 'https://steamcommunity.com/forum/1000/General/' );
<div class="forum_newtopic_box">You need to <a href="https://steamcommunity.com/login/">sign in</a> to do that.</div>"##;

#[test]
fn reads_a_game_forums_new_topic_form() {
    let f = parse_new_topic(FORUM).unwrap();
    assert_eq!((f.owner.as_str(), f.forum_type.as_str(), f.feature.as_str(), f.app_id), ("1000", "General", "0", Some(620)));
    assert_eq!(f.forum_url, "https://steamcommunity.com/app/620/discussions/0/");
    assert!(f.has_form && f.can_post && !f.banned);
    assert_eq!(f.subforums.len(), 3);
    assert_eq!(f.subforums[1].name, "Workshop & Mods");
    assert_eq!(f.default_subforum().unwrap().value, "General_0");
}

#[test]
fn reads_a_group_forum_and_a_signed_out_page() {
    let f = parse_new_topic(GROUP_FORUM).unwrap();
    assert_eq!((f.owner.as_str(), f.feature.as_str(), f.app_id), ("2000", "3", None));
    assert_eq!(f.default_subforum().unwrap().name, "Feedback");
    let out = parse_new_topic(SIGNED_OUT).unwrap();
    assert!(!out.has_form);
    assert_eq!(new_topic_request(&out, "s", "a", "b", None).unwrap_err(), "Sign in to start a discussion");
    assert!(parse_new_topic("<html>no forum here</html>").is_none());
}

#[test]
fn reads_a_group_forums_topics() {
    let page = format!(
        r##"{GROUP_FORUM}
<div class="rightbox forum_list"><div class="rightbox_list_option">
<a href="https://steamcommunity.com/groups/samplegroup/discussions/3/">Feedback</a><div class="forum_list_postcount">7</div></div></div>
<div class="forum_topics"><div class="forum_topic sticky" data-gidforumtopic="42">
<a class="forum_topic_overlay" href="https://steamcommunity.com/groups/samplegroup/discussions/3/42/"></a>
<div class="forum_topic_name">Read this first</div><div class="forum_topic_op">Sample Officer</div>
<div class="forum_topic_reply_count">5</div></div></div>"##
    );
    let p = crate::hub::parse_group_topics(&page).unwrap();
    assert_eq!(p.forum, "Feedback");
    assert_eq!((p.forums.len(), p.forums[0].path.as_str(), p.forums[0].threads), (1, "discussions/3/", 7));
    assert_eq!((p.topics.len(), p.topics[0].id, p.topics[0].title.as_str(), p.topics[0].replies), (1, 42, "Read this first", 5));
    assert!(p.topics[0].pinned);
    assert!(p.new_topic.unwrap().has_form);
    // A group page without a forum address has nothing to read.
    assert!(crate::hub::parse_group_topics("<html></html>").is_err());
}

#[test]
fn builds_the_new_topic_request() {
    let f = parse_new_topic(FORUM).unwrap();
    let (url, fields) = new_topic_request(&f, "abc123", "  A title ", " Some [b]text[/b]\n", Some("Trading_18446744073709551615")).unwrap();
    assert_eq!(url, "https://steamcommunity.com/forum/1000/General/createtopic/0/");
    assert_eq!(
        fields,
        vec![
            ("sessionid", "abc123".to_string()),
            ("appid", "620".to_string()),
            ("topic", "A title".to_string()),
            ("text", "Some [b]text[/b]".to_string()),
            ("subforum", "Trading_18446744073709551615".to_string()),
        ]
    );
    // A group forum: its own feature in the address, no game.
    let g = parse_new_topic(GROUP_FORUM).unwrap();
    let (url, fields) = new_topic_request(&g, "s", "t", "b", None).unwrap();
    assert_eq!(url, "https://steamcommunity.com/forum/2000/General/createtopic/3/");
    assert!(!fields.iter().any(|(k, _)| *k == "appid"));
    assert_eq!(fields.last().unwrap(), &("subforum", "General_3".to_string()));
}

#[test]
fn refuses_a_topic_that_cant_go_through() {
    let f = parse_new_topic(FORUM).unwrap();
    assert_eq!(new_topic_request(&f, "s", "  ", "b", None).unwrap_err(), "Give your discussion a title");
    assert_eq!(new_topic_request(&f, "s", "t", "\n ", None).unwrap_err(), "Write something in your discussion");
    assert!(new_topic_request(&f, "s", "t", "b", Some("Evil_1")).is_err());
    let banned = NewTopicForm { banned: true, ..f.clone() };
    assert_eq!(new_topic_request(&banned, "s", "t", "b", None).unwrap_err(), "You're banned from posting in this forum.");
    let closed = NewTopicForm { can_post: false, ..f.clone() };
    assert_eq!(new_topic_request(&closed, "s", "t", "b", None).unwrap_err(), "You can't post in this forum.");
    let odd = NewTopicForm { owner: "1/../2".into(), ..f };
    assert!(new_topic_request(&odd, "s", "t", "b", None).is_err());
}

#[test]
fn reads_steams_answer_to_a_topic() {
    let url = "https://steamcommunity.com/app/620/discussions/0/5551234/";
    assert_eq!(topic_answer(&format!(r#"{{"success":1,"topic_url":"{url}"}}"#)), Ok(url.to_string()));
    // Only Steam's own site is followed.
    assert!(topic_answer(r#"{"success":1,"topic_url":"https://evil.example/x"}"#).is_err());
    assert_eq!(topic_answer(r#"{"success":2,"error":"That title is <b>too long</b>."}"#).unwrap_err(), "That title is too long .");
    assert_eq!(topic_answer(r#"{"success":84}"#).unwrap_err(), "You're posting too often. Wait a little and try again.");
    assert_eq!(topic_answer(r#"{"success":15}"#).unwrap_err(), "You can't post in this forum.");
    assert_eq!(topic_answer(r#"{"success":5}"#).unwrap_err(), "Steam didn't take your discussion (error 5)");
    assert!(topic_answer("<html>sign in</html>").is_err());
}

const JOIN_FORM: &str = r#"<form name="join_group_form" id="join_group_form" method="POST" action="https://steamcommunity.com/groups/samplegroup">
	<input type="hidden" name="action" value="join"><input type="hidden" name="sessionID" value="sample"></form>"#;

#[test]
fn tells_how_a_group_takes_members() {
    let open = format!(r#"{JOIN_FORM}<div class="grouppage_join_area rightbox"><a href="javascript:document.forms['join_group_form'].submit();" class="btn_green_white_innerfade btn_medium"><span>Join Group</span></a></div>"#);
    let request = format!(r#"{JOIN_FORM}<div class="grouppage_join_area rightbox"><a href="javascript:document.forms['join_group_form'].submit();" class="btn_green_white_innerfade btn_medium"><span>Request To Join</span></a></div>"#);
    let private = format!(r#"{JOIN_FORM}<div class="grouppage_join_area"><img src="https://community.fastly.steamstatic.com/public/images/groups/icon_inviteonly.png">Membership by invitation only</div>"#);
    assert_eq!(parse_join_kind(&open), JoinKind::Open);
    assert_eq!(parse_join_kind(&request), JoinKind::Request);
    assert_eq!(parse_join_kind(&private), JoinKind::InviteOnly);
    assert_eq!(parse_join_kind("<div>a member's view, no join area</div>"), JoinKind::Unknown);
}

#[test]
fn the_button_shows_the_real_state() {
    use ClanRelationship as R;
    assert_eq!(group_button(R::None, JoinKind::Open), GroupButton::Join);
    assert_eq!(group_button(R::None, JoinKind::Request), GroupButton::RequestToJoin);
    assert_eq!(group_button(R::None, JoinKind::InviteOnly), GroupButton::InviteOnly);
    // Where you stand wins over how the group takes members.
    assert_eq!(group_button(R::Member, JoinKind::InviteOnly), GroupButton::Leave);
    assert_eq!(group_button(R::Invited, JoinKind::InviteOnly), GroupButton::Invited);
    assert_eq!(group_button(R::PendingApproval, JoinKind::Request), GroupButton::RequestPending);
    assert_eq!(group_button(R::RequestDenied, JoinKind::Request), GroupButton::RequestDenied);
    assert_eq!(group_button(R::Kicked, JoinKind::Open), GroupButton::Removed);
    assert_eq!(group_button(R::KickAcknowledged, JoinKind::Open), GroupButton::Removed);
    assert_eq!(group_button(R::Blocked, JoinKind::Open), GroupButton::Blocked);
    for (n, r) in [(0, R::None), (1, R::Blocked), (2, R::Invited), (3, R::Member), (4, R::Kicked), (5, R::KickAcknowledged), (6, R::PendingApproval), (7, R::RequestDenied), (9, R::Other(9))] {
        assert_eq!(R::from_u32(n), r);
    }
}

#[test]
fn builds_join_and_leave() {
    let (url, fields) = join_request("samplegroup", "abc").unwrap();
    assert_eq!(url, "https://steamcommunity.com/groups/samplegroup");
    assert_eq!(fields, vec![("action", "join".to_string()), ("sessionID", "abc".to_string())]);
    let (url, _) = join_request(&GROUP.to_string(), "abc").unwrap();
    assert_eq!(url, format!("https://steamcommunity.com/gid/{GROUP}"));
    assert!(join_request("../evil", "abc").is_err());

    let (url, fields) = leave_request(ME, "abc", GROUP).unwrap();
    assert_eq!(url, format!("https://steamcommunity.com/profiles/{ME}/friends/action"));
    assert_eq!(
        fields,
        vec![
            ("sessionid", "abc".to_string()),
            ("steamid", ME.to_string()),
            ("ajax", "1".to_string()),
            ("action", "leave_group".to_string()),
            ("steamids[]", GROUP.to_string()),
        ]
    );
    // A person's id is never "left".
    assert!(leave_request(ME, "abc", ME).is_err());
    assert!(leave_request(0, "abc", GROUP).is_err());
}

#[test]
fn reads_join_and_leave_answers() {
    assert_eq!(join_answer("<div class=\"grouppage_header_name\">A group</div>"), Ok(()));
    let error = r#"<div class="error_ctn"><div id="message"><h1>Sorry!</h1><p class="sectionText">An error was encountered:<br><br></p>
		<h3>You&#39;ve joined too many groups.</h3></div></div>"#;
    assert_eq!(join_answer(error).unwrap_err(), "You've joined too many groups.");
    assert_eq!(leave_answer(r#"{"success":1,"rgCounts":{"cGroups":0}}"#), Ok(()));
    assert!(leave_answer(r#"{"success":2}"#).is_err());
    assert!(leave_answer("<html></html>").is_err());
}

#[test]
fn builds_the_invite_answer() {
    let accept = clan_invite_body(GROUP, true);
    assert_eq!(&accept[..8], &GROUP.to_le_bytes());
    assert_eq!(accept[8], 1);
    assert_eq!(clan_invite_body(GROUP, false)[8], 0);
    assert!(is_clan(GROUP) && !is_clan(ME));
}
