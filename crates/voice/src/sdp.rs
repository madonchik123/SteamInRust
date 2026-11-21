//! The connection descriptions ("SDP") Steam's voice servers swap with us.
//!
//! Steam still uses the older WebRTC layout: a single audio section, with
//! every speaker in it told apart by a stream number (SSRC). Libraries no
//! longer write that layout, so the text is written and read here. Steam
//! wraps it in JSON: `{"type":"offer","sdp":"..."}`.

/// The section name Steam's client uses for audio.
pub const AUDIO_MID: &str = "audio";

/// What we tell the server about ourselves.
#[derive(Debug, Clone)]
pub struct LocalOffer {
    pub session_id: u64,
    pub ufrag: String,
    pub pwd: String,
    /// `sha-256 AA:BB:...`
    pub fingerprint: String,
    pub opus_pt: u8,
    pub ssrc: u32,
}

impl LocalOffer {
    /// The offer text, with the Opus settings Steam's client asks for (loss
    /// recovery on, and almost nothing sent during silence).
    pub fn to_sdp(&self) -> String {
        let LocalOffer { session_id, ufrag, pwd, fingerprint, opus_pt: pt, ssrc } = self;
        [
            "v=0".to_string(),
            format!("o=- {session_id} 2 IN IP4 127.0.0.1"),
            "s=-".into(),
            "t=0 0".into(),
            format!("a=group:BUNDLE {AUDIO_MID}"),
            "a=msid-semantic: WMS steaminrust".into(),
            format!("m=audio 9 UDP/TLS/RTP/SAVPF {pt}"),
            "c=IN IP4 0.0.0.0".into(),
            "a=rtcp:9 IN IP4 0.0.0.0".into(),
            format!("a=ice-ufrag:{ufrag}"),
            format!("a=ice-pwd:{pwd}"),
            "a=ice-options:trickle".into(),
            format!("a=fingerprint:{fingerprint}"),
            "a=setup:actpass".into(),
            format!("a=mid:{AUDIO_MID}"),
            "a=sendrecv".into(),
            "a=rtcp-mux".into(),
            format!("a=rtpmap:{pt} opus/48000/2"),
            format!("a=fmtp:{pt} minptime=10;useinbandfec=1;usedtx=1"),
            format!("a=ssrc:{ssrc} cname:steaminrust"),
            format!("a=ssrc:{ssrc} msid:steaminrust mic"),
            format!("a=ssrc:{ssrc} mslabel:steaminrust"),
            format!("a=ssrc:{ssrc} label:mic"),
            String::new(),
        ]
        .join("\r\n")
    }

    /// The offer the way Steam's service wants it.
    pub fn to_json(&self) -> String {
        wrap("offer", &self.to_sdp())
    }
}

/// Wrap description text in Steam's JSON.
pub fn wrap(kind: &str, sdp: &str) -> String {
    serde_json::json!({ "type": kind, "sdp": sdp }).to_string()
}

/// Which side starts the encryption handshake.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Setup {
    Active,
    Passive,
    ActPass,
}

/// The parts of a description we use.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Description {
    /// `offer` or `answer`.
    pub kind: String,
    pub ufrag: String,
    pub pwd: String,
    pub fingerprint: String,
    pub setup: Setup,
    pub ice_lite: bool,
    /// `candidate:...` lines, without the leading `a=`.
    pub candidates: Vec<String>,
    pub opus_pt: Option<u8>,
    /// Every stream number listed, in order, without repeats.
    pub ssrcs: Vec<u32>,
}

impl Description {
    /// Read Steam's JSON form.
    pub fn from_json(json: &str) -> Result<Self, String> {
        let v: serde_json::Value = serde_json::from_str(json).map_err(|e| format!("not JSON: {e}"))?;
        let kind = v["type"].as_str().ok_or("no type")?;
        let sdp = v["sdp"].as_str().ok_or("no sdp")?;
        Self::parse(kind, sdp)
    }

    pub fn parse(kind: &str, sdp: &str) -> Result<Self, String> {
        let (mut ufrag, mut pwd, mut fingerprint, mut setup) = (None, None, None, None);
        let (mut ice_lite, mut candidates, mut opus_pt, mut ssrcs) = (false, Vec::new(), None, Vec::new());
        for line in sdp.lines().map(str::trim) {
            let Some(attr) = line.strip_prefix("a=") else { continue };
            let (name, value) = attr.split_once(':').unwrap_or((attr, ""));
            match name {
                "ice-lite" => ice_lite = true,
                "ice-ufrag" => ufrag = ufrag.or(Some(value.to_string())),
                "ice-pwd" => pwd = pwd.or(Some(value.to_string())),
                "fingerprint" => fingerprint = fingerprint.or(Some(value.to_string())),
                "setup" => {
                    setup = setup.or(match value {
                        "active" => Some(Setup::Active),
                        "passive" => Some(Setup::Passive),
                        "actpass" => Some(Setup::ActPass),
                        _ => None,
                    })
                }
                "candidate" => candidates.push(attr.to_string()),
                "rtpmap" => {
                    if let Some((pt, codec)) = value.split_once(' ')
                        && codec.to_ascii_lowercase().starts_with("opus/")
                    {
                        opus_pt = opus_pt.or(pt.parse().ok());
                    }
                }
                "ssrc" => {
                    if let Some(id) = value.split(' ').next().and_then(|s| s.parse().ok())
                        && !ssrcs.contains(&id)
                    {
                        ssrcs.push(id);
                    }
                }
                _ => {}
            }
        }
        Ok(Self {
            kind: kind.to_string(),
            ufrag: ufrag.ok_or("no ice-ufrag")?,
            pwd: pwd.ok_or("no ice-pwd")?,
            fingerprint: fingerprint.ok_or("no fingerprint")?,
            setup: setup.ok_or("no setup")?,
            ice_lite,
            candidates,
            opus_pt,
            ssrcs,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ANSWER: &str = "v=0\r\no=- 1 2 IN IP4 127.0.0.1\r\ns=-\r\nt=0 0\r\na=ice-lite\r\na=group:BUNDLE audio\r\nm=audio 27015 UDP/TLS/RTP/SAVPF 111\r\nc=IN IP4 203.0.113.9\r\na=candidate:1 1 udp 2130706431 203.0.113.9 27015 typ host\r\na=ice-ufrag:srvU\r\na=ice-pwd:serverpassword0123456789\r\na=fingerprint:sha-256 AB:CD\r\na=setup:passive\r\na=mid:audio\r\na=sendrecv\r\na=rtcp-mux\r\na=rtpmap:111 opus/48000/2\r\na=fmtp:111 minptime=10;useinbandfec=1\r\na=ssrc:1001 cname:a\r\na=ssrc:1001 msid:Peer-1-111- t\r\na=ssrc:2002 cname:b\r\n";

    #[test]
    fn reads_a_server_answer() {
        let d = Description::from_json(&wrap("answer", ANSWER)).unwrap();
        assert_eq!(d.kind, "answer");
        assert_eq!((d.ufrag.as_str(), d.pwd.as_str()), ("srvU", "serverpassword0123456789"));
        assert_eq!(d.fingerprint, "sha-256 AB:CD");
        assert_eq!(d.setup, Setup::Passive);
        assert!(d.ice_lite);
        assert_eq!(d.candidates, ["candidate:1 1 udp 2130706431 203.0.113.9 27015 typ host"]);
        assert_eq!(d.opus_pt, Some(111));
        assert_eq!(d.ssrcs, [1001, 2002], "every speaker once, in order");
    }

    #[test]
    fn a_missing_key_part_is_an_error() {
        let no_fingerprint = ANSWER.replace("a=fingerprint:sha-256 AB:CD\r\n", "");
        assert!(Description::parse("answer", &no_fingerprint).unwrap_err().contains("fingerprint"));
        assert!(Description::from_json("not json").is_err());
    }

    #[test]
    fn our_offer_has_what_steam_asks_for_and_reads_back() {
        let o = LocalOffer { session_id: 7, ufrag: "uf".into(), pwd: "pw".into(), fingerprint: "sha-256 01:02".into(), opus_pt: 111, ssrc: 42 };
        let text = o.to_sdp();
        assert!(text.contains("a=fmtp:111 minptime=10;useinbandfec=1;usedtx=1\r\n"));
        assert!(text.contains("a=setup:actpass\r\n"));
        assert_eq!(text.matches("m=").count(), 1, "one audio section");
        let d = Description::from_json(&o.to_json()).unwrap();
        assert_eq!((d.kind.as_str(), d.opus_pt, d.setup), ("offer", Some(111), Setup::ActPass));
        assert_eq!(d.ssrcs, [42]);
    }
}
