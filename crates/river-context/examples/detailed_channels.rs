//! Detailed channel examples with moves and moments compression.
//!
//! Run with: cargo run --example detailed_channels

use river_context::{
    build_context, Author, Channel, ChannelContext, ChatMessage, ContextRequest, Embedding, Flash,
    Moment, Move, OpenAIMessage,
};
use std::fs::File;
use std::io::Write;

/// Snowflake ID from timestamp (minutes from epoch, for readability)
fn id(minutes: u64) -> String {
    let micros = minutes * 60 * 1_000_000;
    let snowflake: u128 = (micros as u128) << 64;
    snowflake.to_string()
}

fn channel(adapter: &str, name: &str) -> Channel {
    Channel {
        adapter: adapter.into(),
        id: format!("chan_{}", name.to_lowercase().replace(" ", "_")),
        name: Some(name.into()),
    }
}

fn msg(id_minutes: u64, author: &str, bot: bool, content: &str) -> ChatMessage {
    let hour = (id_minutes / 60) % 24;
    let min = id_minutes % 60;
    ChatMessage {
        id: id(id_minutes),
        timestamp: format!("2026-04-01T{:02}:{:02}:00Z", hour, min),
        author: Author {
            id: format!("user_{}", author.to_lowercase()),
            name: author.into(),
            bot,
        },
        content: content.into(),
    }
}

fn mv(id_minutes: u64, content: &str, start: u64, end: u64) -> Move {
    Move {
        id: id(id_minutes),
        content: content.into(),
        message_range: (id(start), id(end)),
    }
}

fn moment(id_minutes: u64, content: &str, start_move: u64, end_move: u64) -> Moment {
    Moment {
        id: id(id_minutes),
        content: content.into(),
        move_range: (id(start_move), id(end_move)),
    }
}

fn flash(id_minutes: u64, from: &str, content: &str) -> Flash {
    Flash {
        id: id(id_minutes),
        from: from.into(),
        content: content.into(),
        expires_at: "2099-01-01T00:00:00Z".into(),
    }
}

fn embedding(id_minutes: u64, content: &str, source: &str) -> Embedding {
    Embedding {
        id: id(id_minutes),
        content: content.into(),
        source: source.into(),
        expires_at: "2099-01-01T00:00:00Z".into(),
    }
}

/// Channel 1: #dev - A debugging session about authentication
fn dev_channel() -> ChannelContext {
    // === RAW MESSAGES (50 messages over ~2 hours) ===
    let messages = vec![
        // Opening (9:00-9:10)
        msg(540, "alice", false, "hey, auth is broken again"),
        msg(541, "River", true, "What's happening? Can you describe the symptoms?"),
        msg(542, "alice", false, "users getting logged out randomly, maybe every 10-15 min"),
        msg(543, "alice", false, "no pattern i can see"),
        msg(544, "River", true, "Let me check the token refresh logic. What's the current TTL setting?"),
        msg(545, "bob", false, "i set it to 15 min last week, was that wrong?"),
        msg(546, "River", true, "That might be it. If the refresh happens at exactly 15 min but there's any latency..."),
        msg(547, "alice", false, "oh no"),
        msg(548, "bob", false, "should we bump it up?"),
        msg(549, "River", true, "Let me look at the refresh code first. Where's that implemented?"),

        // Investigation (9:10-9:30)
        msg(550, "alice", false, "src/auth/refresh.rs"),
        msg(551, "River", true, "Found it. The refresh triggers at TTL - 30 seconds, but that's not enough buffer."),
        msg(552, "bob", false, "what buffer do we need?"),
        msg(553, "River", true, "I'd recommend TTL - 2 minutes minimum. Network latency + processing time."),
        msg(554, "alice", false, "makes sense"),
        msg(555, "River", true, "Also noticed the refresh endpoint doesn't retry on failure."),
        msg(556, "bob", false, "that's bad"),
        msg(557, "River", true, "Yes. If the refresh fails once, the user gets logged out."),
        msg(558, "alice", false, "we need retry logic"),
        msg(559, "River", true, "Agreed. I'll draft a fix. Give me a few minutes."),

        // Proposal (9:30-9:45)
        msg(560, "River", true, "Here's what I'm thinking:\n1. Change refresh window to TTL - 2min\n2. Add 3 retries with exponential backoff\n3. Add a 'soft logout' state that shows a refresh prompt instead of hard logout"),
        msg(561, "alice", false, "i like the soft logout idea"),
        msg(562, "bob", false, "what's exponential backoff?"),
        msg(563, "River", true, "Retry after 1s, then 2s, then 4s. Prevents hammering the server."),
        msg(564, "bob", false, "got it"),
        msg(565, "alice", false, "can we also log these failures? for debugging"),
        msg(566, "River", true, "Good call. I'll add structured logging with correlation IDs."),
        msg(567, "bob", false, "what's a correlation id"),
        msg(568, "River", true, "A unique ID that follows a request through the system. Makes tracing easier."),
        msg(569, "alice", false, "we should have added that ages ago"),

        // Implementation discussion (9:45-10:00)
        msg(570, "River", true, "I've written the refresh changes. PR #847. Can someone review?"),
        msg(571, "bob", false, "looking now"),
        msg(572, "alice", false, "me too"),
        msg(573, "bob", false, "line 45 - shouldn't the retry count be configurable?"),
        msg(574, "River", true, "Good point. I'll add it to the config."),
        msg(575, "alice", false, "also line 78, the error message is confusing"),
        msg(576, "River", true, "Which part?"),
        msg(577, "alice", false, "\"token refresh failed\" doesn't say why"),
        msg(578, "River", true, "True. I'll include the underlying error."),
        msg(579, "bob", false, "otherwise lgtm"),

        // Testing (10:00-10:15)
        msg(580, "River", true, "Updated. Also added unit tests for the retry logic."),
        msg(581, "alice", false, "nice, running the test suite now"),
        msg(582, "bob", false, "should we do a load test too?"),
        msg(583, "River", true, "Yes, especially for the soft logout UI. What's our load testing setup?"),
        msg(584, "alice", false, "we use k6, scripts are in /tests/load"),
        msg(585, "River", true, "I'll add a scenario for auth refresh under load."),
        msg(586, "bob", false, "tests passed locally btw"),
        msg(587, "alice", false, "same here"),
        msg(588, "River", true, "Great. Load test script is ready. Running now."),
        msg(589, "alice", false, "fingers crossed"),

        // Resolution (10:15-10:30)
        msg(590, "River", true, "Load test passed. 10k concurrent users, no auth failures."),
        msg(591, "bob", false, "nice!"),
        msg(592, "alice", false, "merging"),
        msg(593, "River", true, "One more thing - should we notify users about the fix?"),
        msg(594, "alice", false, "good idea, i'll write a changelog entry"),
        msg(595, "bob", false, "deployed to staging"),
        msg(596, "River", true, "I'll monitor the logs for the next hour."),
        msg(597, "alice", false, "thanks river, this was a good session"),
        msg(598, "bob", false, "yeah really helpful"),
        msg(599, "River", true, "Happy to help. Let me know if anything else comes up."),
    ];

    // === MOVES (compressed from messages) ===
    let moves = vec![
        mv(541, "Problem report: users randomly logged out every 10-15 min", 540, 543),
        mv(549, "Initial hypothesis: TTL set to 15 min, latency causing race condition", 544, 549),
        mv(555, "Investigation: refresh triggers at TTL-30s (insufficient buffer), no retry on failure", 550, 559),
        mv(566, "Proposal: 3-part fix - larger refresh window, retry with backoff, soft logout state. Added logging with correlation IDs.", 560, 569),
        mv(579, "Code review: made retry count configurable, improved error messages", 570, 579),
        mv(589, "Testing: unit tests added, load test with k6 prepared", 580, 589),
        mv(599, "Resolution: load test passed (10k users), merged, deployed to staging", 590, 599),
    ];

    // === MOMENTS (compressed from moves) ===
    let moments = vec![
        moment(600, "Auth token refresh debugging session. Root cause: 15-min TTL with only 30s refresh buffer + no retry logic. Fix: 2-min buffer, exponential backoff retries, soft logout UI. Load tested at 10k concurrent users. Deployed to staging. Duration: ~90 minutes.", 541, 599),
    ];

    ChannelContext {
        channel: channel("discord", "dev"),
        moments,
        moves,
        messages,  // In practice, raw messages would be trimmed after compression
        embeddings: vec![
            embedding(555, "Auth refresh should happen at TTL minus a safe buffer (2+ minutes) to account for network latency and processing time.", "notes/auth-patterns.md"),
        ],
        inbox: vec![],
    }
}

/// Channel 2: #design - A UI/UX discussion about onboarding flow
fn design_channel() -> ChannelContext {
    let messages = vec![
        // Opening (11:00-11:10)
        msg(660, "carol", false, "need help with the onboarding redesign"),
        msg(661, "River", true, "Sure! What's the current state and what are you trying to improve?"),
        msg(662, "carol", false, "users dropping off at step 3 of 5"),
        msg(663, "carol", false, "that's the 'connect accounts' step"),
        msg(664, "River", true, "What's the current drop-off rate there?"),
        msg(665, "carol", false, "about 40%"),
        msg(666, "River", true, "That's significant. What does step 3 ask users to do exactly?"),
        msg(667, "carol", false, "connect their google/github/slack accounts"),
        msg(668, "dan", false, "we require at least one connection to proceed"),
        msg(669, "River", true, "Ah, that might be the issue. Requiring account connection creates friction."),

        // Problem exploration (11:10-11:25)
        msg(670, "carol", false, "but we need those integrations for the product to work"),
        msg(671, "River", true, "Do you need them immediately, or can users connect later?"),
        msg(672, "dan", false, "technically later works, but then they miss the value"),
        msg(673, "River", true, "What if we showed the value first, then asked for the connection?"),
        msg(674, "carol", false, "like a preview?"),
        msg(675, "River", true, "Exactly. 'Here's what you'll get when you connect' with sample data."),
        msg(676, "dan", false, "interesting"),
        msg(677, "carol", false, "but that's more dev work"),
        msg(678, "River", true, "True. Let me think about lighter options."),
        msg(679, "dan", false, "we could just make it optional"),

        // Options discussion (11:25-11:40)
        msg(680, "River", true, "Here are three options:\n\nA) Make connection optional, show value later\nB) Preview with sample data (more dev work)\nC) Reorder flow - show value in steps 1-2, ask for connection in step 4"),
        msg(681, "carol", false, "i like C"),
        msg(682, "dan", false, "C makes sense, what would steps 1-2 show?"),
        msg(683, "River", true, "Step 1: personalization (name, role, goals)\nStep 2: quick demo of core features with fake data\nStep 3: 'ready to make it real?' - connect accounts\nStep 4: workspace setup\nStep 5: invite team"),
        msg(684, "carol", false, "oh that's much better"),
        msg(685, "dan", false, "the fake data demo is basically option B though"),
        msg(686, "River", true, "Simpler version - just screenshots or a video, not interactive."),
        msg(687, "carol", false, "we have marketing videos we could use"),
        msg(688, "dan", false, "true"),
        msg(689, "River", true, "Even simpler: animated GIFs showing the workflow. No video player needed."),

        // Refinement (11:40-11:55)
        msg(690, "carol", false, "i can make those gifs today"),
        msg(691, "River", true, "Perfect. For the connection step, let's also add a 'skip for now' option."),
        msg(692, "dan", false, "with a reminder later?"),
        msg(693, "River", true, "Yes. In-app prompt after they've used the product for 10 minutes."),
        msg(694, "carol", false, "smart, they'll have seen the value by then"),
        msg(695, "dan", false, "what about the people who do connect immediately?"),
        msg(696, "River", true, "Fast path - skip the demo, go straight to workspace setup."),
        msg(697, "carol", false, "two paths then"),
        msg(698, "River", true, "Right. Connected users: personalize → connect → setup → invite\nSkipped users: personalize → demo → skip → setup → invite → (later prompt)"),
        msg(699, "dan", false, "that's clean"),

        // Wireframes (11:55-12:10)
        msg(700, "carol", false, "should i wireframe this?"),
        msg(701, "River", true, "Yes. Focus on the decision point at step 3 - make 'skip' visible but not dominant."),
        msg(702, "dan", false, "maybe a secondary button style?"),
        msg(703, "River", true, "Exactly. Primary: 'Connect Google' etc. Secondary: 'Skip for now'"),
        msg(704, "carol", false, "with microcopy explaining what they'll miss?"),
        msg(705, "River", true, "Light touch. 'You can connect anytime from Settings' - not scary."),
        msg(706, "dan", false, "no guilt tripping"),
        msg(707, "River", true, "Right. Trust that the product sells itself once they're in."),
        msg(708, "carol", false, "ok i'll have wireframes by end of day"),
        msg(709, "River", true, "Great. Share in #design-reviews when ready."),
    ];

    let moves = vec![
        mv(669, "Problem: 40% drop-off at step 3 (required account connection)", 660, 669),
        mv(679, "Exploration: connection required for value, but timing is flexible", 670, 679),
        mv(689, "Options proposed: A) optional connection, B) preview with sample data, C) reorder flow. Team preferred C.", 680, 689),
        mv(698, "Refinement: two-path flow - connected users fast-track, skippers get demo + later prompt", 690, 699),
        mv(709, "Decision: reordered flow with skip option, animated GIF demo, secondary button styling. Wireframes EOD.", 700, 709),
    ];

    let moments = vec![
        moment(710, "Onboarding redesign for step 3 drop-off (40%). Solution: reorder flow to show value before asking for account connection. Two paths: immediate connectors fast-track, skippers get GIF demo + deferred prompt. Wireframes in progress.", 669, 709),
    ];

    ChannelContext {
        channel: channel("discord", "design"),
        moments,
        moves,
        messages,
        embeddings: vec![
            embedding(680, "Onboarding best practice: show value before asking for commitment. Users who see benefits first convert at 2-3x higher rates.", "embeddings/onboarding-research.md"),
        ],
        inbox: vec![],
    }
}

/// Channel 3: #random - Casual chat (other channel, only moments+moves shown)
fn random_channel() -> ChannelContext {
    // Fewer messages since this will only show summaries
    let messages = vec![
        msg(480, "alice", false, "anyone want coffee?"),
        msg(481, "bob", false, "yes please"),
        msg(482, "carol", false, "i'll take a cold brew"),
        msg(483, "dan", false, "same"),
        msg(485, "alice", false, "ok 4 coffees, back in 10"),
        msg(495, "alice", false, "coffee delivered ☕"),
        msg(496, "bob", false, "ty!"),
        msg(500, "dan", false, "did anyone see the game last night?"),
        msg(501, "bob", false, "yeah! that last play was wild"),
        msg(502, "carol", false, "i missed it, was working late"),
        msg(503, "dan", false, "highlights are on youtube"),
        msg(510, "alice", false, "lunch plans?"),
        msg(511, "bob", false, "thai place on 5th?"),
        msg(512, "carol", false, "im in"),
        msg(513, "dan", false, "same"),
    ];

    let moves = vec![
        mv(496, "Coffee run coordination - alice got 4 coffees", 480, 496),
        mv(503, "Sports chat - game highlights discussion", 500, 503),
        mv(513, "Lunch plans - thai place on 5th, group going", 510, 513),
    ];

    let moments = vec![
        moment(515, "Casual team coordination: coffee run, sports chat, lunch plans (thai on 5th)", 496, 513),
    ];

    ChannelContext {
        channel: channel("discord", "random"),
        moments,
        moves,
        messages,
        embeddings: vec![], // Other channels don't get embeddings in output
        inbox: vec![],
    }
}

fn format_output(request: &ContextRequest, response: &river_context::ContextResponse) -> String {
    let mut out = String::new();

    out.push_str(&format!("{}\n", "=".repeat(80)));
    out.push_str("INPUT SUMMARY\n");
    out.push_str(&format!("{}\n\n", "=".repeat(80)));

    for (i, ch) in request.channels.iter().enumerate() {
        let role = match i {
            0 => "CURRENT",
            1 => "LAST",
            _ => "OTHER",
        };
        out.push_str(&format!("Channel {} [{}]: #{}\n", i, role, ch.channel.name.as_ref().unwrap()));
        out.push_str(&format!("  Messages: {}\n", ch.messages.len()));
        out.push_str(&format!("  Moves: {}\n", ch.moves.len()));
        out.push_str(&format!("  Moments: {}\n", ch.moments.len()));
        out.push_str(&format!("  Embeddings: {}\n", ch.embeddings.len()));
    }

    out.push_str(&format!("\nFlashes: {}\n", request.flashes.len()));
    out.push_str(&format!("History items: {}\n", request.history.len()));
    out.push_str(&format!("Max tokens: {}\n", request.max_tokens));

    out.push_str(&format!("\n{}\n", "=".repeat(80)));
    out.push_str(&format!("OUTPUT ({} tokens estimated)\n", response.estimated_tokens));
    out.push_str(&format!("{}\n\n", "=".repeat(80)));

    for (i, msg) in response.messages.iter().enumerate() {
        let role = &msg.role;
        let content = msg.content.as_ref().map(|c| c.as_str()).unwrap_or("[no content]");

        // Determine message type from content
        let msg_type = if content.starts_with("[Moment:") {
            "MOMENT"
        } else if content.starts_with("[Move:") {
            "MOVE"
        } else if content.starts_with("[Flash") {
            "FLASH"
        } else if content.starts_with("[Reference:") {
            "EMBED"
        } else if content.starts_with("[Chat:") {
            "CHAT"
        } else {
            "HISTORY"
        };

        out.push_str(&format!("--- Message {} [{}/{}] ---\n", i + 1, role, msg_type));
        out.push_str(content);
        out.push_str("\n\n");
    }

    out
}

fn main() -> std::io::Result<()> {
    let mut results = String::new();

    results.push_str("DETAILED CHANNEL EXAMPLES - build_context OUTPUT\n");
    results.push_str(&format!("Generated: 2026-04-01\n\n"));

    // === SCENARIO 1: Current=#dev, Last=#design, Other=#random ===
    results.push_str(&format!("\n{}\n", "#".repeat(80)));
    results.push_str("# SCENARIO 1: Debugging session is current, design discussion is last\n");
    results.push_str(&format!("{}\n", "#".repeat(80)));

    let request1 = ContextRequest {
        channels: vec![
            dev_channel(),      // Current - full messages shown
            design_channel(),   // Last - moments + moves + embeddings
            random_channel(),   // Other - moments + moves only
        ],
        flashes: vec![
            flash(575, "spectator", "Reminder: auth token patterns documented in notes/auth-patterns.md"),
            flash(695, "spectator", "Consider: similar drop-off issue was solved in mobile app redesign Q3"),
        ],
        history: vec![
            OpenAIMessage::system("You are River, a helpful engineering assistant."),
            OpenAIMessage::system("Your identity: collaborative, thorough, explains reasoning."),
        ],
        max_tokens: 50000,
        now: "2026-04-01T12:00:00Z".into(),
    };

    match build_context(request1.clone()) {
        Ok(response) => {
            results.push_str(&format_output(&request1, &response));
        }
        Err(e) => {
            results.push_str(&format!("ERROR: {:?}\n", e));
        }
    }

    // === SCENARIO 2: Current=#design (switched from #dev) ===
    results.push_str(&format!("\n{}\n", "#".repeat(80)));
    results.push_str("# SCENARIO 2: Just switched to design channel\n");
    results.push_str(&format!("{}\n", "#".repeat(80)));

    let request2 = ContextRequest {
        channels: vec![
            design_channel(),   // Current now
            dev_channel(),      // Was current, now last
            random_channel(),   // Still other
        ],
        flashes: vec![
            flash(700, "spectator", "Auth fix deployed - team may want status update"),
        ],
        history: vec![
            OpenAIMessage::system("You are River, a helpful engineering assistant."),
            OpenAIMessage::assistant("I've finished helping with the auth token fix in #dev. The PR is merged and deployed to staging."),
        ],
        max_tokens: 50000,
        now: "2026-04-01T12:15:00Z".into(),
    };

    match build_context(request2.clone()) {
        Ok(response) => {
            results.push_str(&format_output(&request2, &response));
        }
        Err(e) => {
            results.push_str(&format!("ERROR: {:?}\n", e));
        }
    }

    // === SCENARIO 3: Only moments (simulating older context) ===
    results.push_str(&format!("\n{}\n", "#".repeat(80)));
    results.push_str("# SCENARIO 3: Compressed context (only moments, no raw messages)\n");
    results.push_str(&format!("{}\n", "#".repeat(80)));

    // Strip messages, keep only summaries
    let mut dev_compressed = dev_channel();
    dev_compressed.messages = vec![];  // Messages have been compressed away
    dev_compressed.moves = vec![];     // Only moments remain

    let mut design_compressed = design_channel();
    design_compressed.messages = vec![];
    design_compressed.moves = vec![];

    let mut random_compressed = random_channel();
    random_compressed.messages = vec![];
    random_compressed.moves = vec![];

    let request3 = ContextRequest {
        channels: vec![
            dev_compressed,
            design_compressed,
            random_compressed,
        ],
        flashes: vec![],
        history: vec![
            OpenAIMessage::system("You are River, a helpful engineering assistant."),
            OpenAIMessage::user("What did we work on today?"),
        ],
        max_tokens: 50000,
        now: "2026-04-01T18:00:00Z".into(),
    };

    match build_context(request3.clone()) {
        Ok(response) => {
            results.push_str(&format_output(&request3, &response));
        }
        Err(e) => {
            results.push_str(&format!("ERROR: {:?}\n", e));
        }
    }

    // Write to file
    let output_path = "target/build_context_detailed_output.txt";
    let mut file = File::create(output_path)?;
    file.write_all(results.as_bytes())?;

    println!("Results written to: {}", output_path);
    println!("\n{}", results);

    Ok(())
}
