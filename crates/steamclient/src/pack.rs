//! Steam's callback structs are packed to 8 bytes on Windows and to 4 bytes
//! on macOS and Linux (`VALVE_CALLBACK_PACK_SMALL` in the Steamworks
//! headers). The code builds every struct with Windows' layout (offsets
//! written out by hand); on macOS and Linux [`small`] rewrites the ones whose
//! layout differs before a game reads them. Only structs with a 64-bit field
//! that 4-byte packing moves, or whose size changes, are listed.

/// A field: (size, natural alignment). Byte arrays have alignment 1; a
/// nested struct is one field with its own alignment.
type Field = (usize, usize);

const U8: Field = (1, 1);
const U16: Field = (2, 2);
const U32: Field = (4, 4);
const U64: Field = (8, 8);
const fn bytes(n: usize) -> Field {
    (n, 1)
}

/// SteamUGCDetails_t (also returned by ISteamUGC::GetQueryUGCResult).
pub const UGC_DETAILS: &[Field] = &[
    U64,          // m_nPublishedFileId
    U32,          // m_eResult
    U32,          // m_eFileType
    U32,          // m_nCreatorAppID
    U32,          // m_nConsumerAppID
    bytes(129),   // m_rgchTitle
    bytes(8000),  // m_rgchDescription
    U64,          // m_ulSteamIDOwner
    U32,          // m_rtimeCreated
    U32,          // m_rtimeUpdated
    U32,          // m_rtimeAddedToUserList
    U32,          // m_eVisibility
    U8,           // m_bBanned
    U8,           // m_bAcceptedForUse
    U8,           // m_bTagsTruncated
    bytes(1025),  // m_rgchTags
    U64,          // m_hFile
    U64,          // m_hPreviewFile
    bytes(260),   // m_pchFileName
    U32,          // m_nFileSize
    U32,          // m_nPreviewFileSize
    bytes(256),   // m_rgchURL
    U32,          // m_unVotesUp
    U32,          // m_unVotesDown
    U32,          // m_flScore
    U32,          // m_unNumChildren
];

/// LeaderboardEntry_t (ISteamUserStats::GetDownloadedLeaderboardEntry).
pub const LEADERBOARD_ENTRY: &[Field] = &[U64, U32, U32, U32, U64];

/// The fields of a callback or call-result struct whose layout differs.
fn fields(callback: i32) -> Option<Vec<Field>> {
    Some(match callback {
        143 => vec![U64, U32, U64],            // ValidateAuthTicketResponse_t
        334 => vec![U64, U32, U32, U32],       // AvatarImageLoaded_t
        504 => vec![U64, U32, U8, U32],        // LobbyEnter_t
        505 => vec![U64, U64, U8],             // LobbyDataUpdate_t
        506 => vec![U64, U64, U64, U32],       // LobbyChatUpdate_t
        509 => vec![U64, U64, U32, U16],       // LobbyGameCreated_t
        512 => vec![U64, U64, U8],             // LobbyKicked_t
        513 => vec![U32, U64],                 // LobbyCreated_t
        1101 => vec![U64, U32, U64],           // UserStatsReceived_t
        1102 => vec![U64, U32],                // UserStatsStored_t
        1103 => vec![U64, U8, bytes(128), U32, U32], // UserAchievementStored_t
        1104 => vec![U64, U8],                 // LeaderboardFindResult_t
        1105 => vec![U64, U64, U32],           // LeaderboardScoresDownloaded_t
        1106 => vec![U8, U64, U32, U8, U32, U32], // LeaderboardScoreUploaded_t
        1110 => vec![U64, U32],                // GlobalAchievementPercentagesReady_t
        1111 => vec![U32, U64],                // LeaderboardUGCSet_t
        1112 => vec![U64, U32],                // GlobalStatsReceived_t
        1800 | 1801 => vec![U32, U64],         // GSStatsReceived_t, GSStatsStored_t
        1203 => vec![U64, U8],                 // P2PSessionConnectFail_t
        1221 => vec![U32, (696, 8), U32],      // SteamNetConnectionStatusChangedCallback_t
        1307 => vec![U32, U64, bytes(260)],    // RemoteStorageFileShareResult_t
        1313 | 1315 => vec![U32, U64],         // RemoteStorage(Un)SubscribePublishedFileResult_t
        1321 | 1322 => vec![U64, U32],         // RemoteStoragePublishedFile(Un)Subscribed_t
        1332 => vec![U64, U32, U32, U32],      // RemoteStorageFileReadAsyncComplete_t
        2101 => vec![U32, U64, U8, U32, U32],  // HTTPRequestCompleted_t
        3403 => vec![U32, U64, U8],            // CreateItemResult_t
        3405 => vec![U32, U64, U64, U64],      // ItemInstalled_t
        3406 => vec![U32, U64, U32],           // DownloadItemResult_t
        4703 => vec![U32, U64, U32, U8],       // SteamInventoryEligiblePromoItemDefIDs_t
        4704 => vec![U32, U64, U64],           // SteamInventoryStartPurchaseResult_t
        _ => return None,
    })
}

/// Offsets of `fields` and the padded size, with alignment capped at `pack`.
fn layout(fields: &[Field], pack: usize) -> (Vec<usize>, usize) {
    let mut at = 0usize;
    let mut max = 1usize;
    let offsets = fields
        .iter()
        .map(|&(size, align)| {
            let a = align.min(pack);
            max = max.max(a);
            at = at.next_multiple_of(a);
            let o = at;
            at += size;
            o
        })
        .collect();
    (offsets, at.next_multiple_of(max))
}

fn struct_size(fields: &[Field], pack: usize) -> usize {
    layout(fields, pack).1
}

/// Rewrite a struct laid out with 8-byte packing into 4-byte packing.
pub fn repack(fields: &[Field], data: &[u8]) -> Vec<u8> {
    let (from, size8) = layout(fields, 8);
    let (to, size4) = layout(fields, 4);
    // Anything after the described fields (a newer SDK's additions) follows them.
    let extra = data.len().saturating_sub(size8);
    let mut out = vec![0u8; size4 + extra];
    for ((&(size, _), &f), &t) in fields.iter().zip(&from).zip(&to) {
        if let Some(src) = data.get(f..f + size) {
            out[t..t + size].copy_from_slice(src);
        }
    }
    if extra > 0 {
        out[size4..].copy_from_slice(&data[size8..]);
    }
    out
}

/// A callback or call result in this system's layout.
pub fn small(callback: i32, data: Vec<u8>) -> Vec<u8> {
    if cfg!(windows) {
        return data;
    }
    // SteamUGCRequestUGCDetailsResult_t: SteamUGCDetails_t, then a bool.
    if callback == 3402 {
        let n = struct_size(UGC_DETAILS, 8);
        let mut out = repack(UGC_DETAILS, &data[..n.min(data.len())]);
        out.extend_from_slice(data.get(n..n + 1).unwrap_or(&[0]));
        out.resize(out.len().next_multiple_of(4), 0);
        return out;
    }
    match fields(callback) {
        Some(f) => repack(&f, &data),
        None => data,
    }
}

/// An output struct (not a callback) in this system's layout.
pub fn out(fields: &[Field], data: Vec<u8>) -> Vec<u8> {
    if cfg!(windows) { data } else { repack(fields, &data) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn offsets_follow_packing() {
        // UserStatsReceived_t: game id, result, user.
        assert_eq!(layout(&[U64, U32, U64], 8), (vec![0, 8, 16], 24));
        assert_eq!(layout(&[U64, U32, U64], 4), (vec![0, 8, 12], 20));
        // LobbyEnter_t keeps its offsets but loses the tail padding.
        assert_eq!(layout(&[U64, U32, U8, U32], 4), (vec![0, 8, 12, 16], 20));
        // The sizes the Windows code writes.
        assert_eq!(struct_size(UGC_DETAILS, 8), 9776);
        assert_eq!(struct_size(LEADERBOARD_ENTRY, 8), 32);
        assert_eq!(struct_size(&fields(1221).unwrap(), 8), 712);
        // SteamUGCDetails_t on macOS: the owner moves from 8160 to 8156, the file handle from 9216 to 9208.
        let (o, size) = layout(UGC_DETAILS, 4);
        assert_eq!((o[7], o[16], size), (8156, 9208, 9764));
    }

    #[cfg(not(windows))]
    #[test]
    fn ugc_details_result() {
        let mut win = vec![0u8; 9784];
        win[8160..8168].copy_from_slice(&7u64.to_le_bytes());
        win[9776] = 1;
        let mac = small(3402, win);
        assert_eq!(mac.len(), 9768);
        assert_eq!(u64::from_le_bytes(mac[8156..8164].try_into().unwrap()), 7);
        assert_eq!(mac[9764], 1);
    }

    #[test]
    fn moves_fields() {
        let mut win = vec![0u8; 24];
        win[0..8].copy_from_slice(&480u64.to_le_bytes());
        win[8..12].copy_from_slice(&1i32.to_le_bytes());
        win[16..24].copy_from_slice(&76561198000000001u64.to_le_bytes());
        let mac = repack(&[U64, U32, U64], &win);
        assert_eq!(mac.len(), 20);
        assert_eq!(u64::from_le_bytes(mac[12..20].try_into().unwrap()), 76561198000000001);
        assert_eq!(i32::from_le_bytes(mac[8..12].try_into().unwrap()), 1);
    }
}
