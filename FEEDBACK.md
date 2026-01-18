# User Feedback Log

All user messages and feedback are logged here for context preservation.

## Session 1 - Project Creation

**Date**: 2026-01-17

**User request**: Create zengif crate for GIF handling with:
- Animation, disposal, transparency support (all 3 combined)
- Streaming decode and encode
- Round-trip with timing and metadata preserved
- Memory tracking, estimation, and bounds
- `whereat` + `enough` crate integration
- Size limit enforcement
- Server-side zero-trust use case
- Learn from: gif-dispose, gif, imageflow gif handling (flawed), gifski, pngquant, gifski-lite

**Notes**: imageflow's gif handling has flawed animation transparency support and produces terrible output sizes.
