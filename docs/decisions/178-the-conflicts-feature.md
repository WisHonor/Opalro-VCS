# 178 — The `conflicts` feature: how a project comes to keep conflicts

- **Status:** Accepted
- **Date:** 2026-11-14
- **Agenda topic:** none (milestone 9 planning)
- **Decided by:** Claude, at the owner's request while the owner was away. The owner can reverse any part.
- **Refines:** [decision 008](008-layout-and-settings.md) (a newer Opalro never upgrades a project's format by itself; it suggests, and `opalro setup` performs); [decision 121](121-format-1-freezes-at-v0.1.md) (anything new on disk arrives as a `feature` line, which a project gains only when its user asks, through `opalro setup`); [decision 148](148-what-format-1s-spec-says-before-it-freezes.md) (v0.1 refuses a project that lists `feature conflicts`); [decision 157](157-v0.2-in-four-milestones.md) ("only conflicts need something new on disk, and milestone 9 decides how that arrives")

## Context

**In plain words.** A conflict record is the first thing v0.2 stores that v0.1 can't read. Format 1 has a way to say so: a project's `.opalro/format` file lists the `features` it uses, and an Opalro that meets a feature it doesn't know refuses the project cleanly and asks to be updated. v0.1 already refuses `feature conflicts` that way. So the moment a project lists that feature, v0.1 can no longer open it.

The owner decided twice how features arrive: a newer Opalro "never upgrades their format by itself; it suggests upgrades, and `bolt setup` performs them" (decision 008), and a project gains a feature "only when its user asks, through `bolt setup`" (decision 121). The one exception was made on purpose and closed: `feature chunks`, added without asking before format 1 froze.

Today `opalro setup` only writes the project's rules; it has no step that changes the format. And the code that writes the `format` file writes a fixed text, so it would wipe out any feature but `chunks`.

Three kinds of project meet v0.2: one made by v0.2, one made by v0.1 that never has a conflict, and one made by v0.1 where a combine now needs to keep one.

## Options

**A project made by v0.1, when a combine needs to keep a conflict**

- **A. The combine stops and names `opalro setup --upgrade`.** The owner's rule as written. One more command, once in a project's life, and the person sees plainly what it costs (v0.1 can't open the project afterwards) before anything is stored.
- **B. The combine asks on the spot** and goes on after a yes. One step fewer; it is still the user who says yes, but not "through `opalro setup`", so it would change a rule the owner set twice.
- **C. Add the feature silently,** as `chunks` was. Against both decisions.

**A project made by v0.2**

- **A. Lists the feature from birth,** as new projects list `chunks`. v0.1 can't open it; it is a v0.2 project.
- **B. Starts without it** and upgrades at its first conflict. Every project then meets the extra step once, to stay openable by a version nobody would go back to.

## Decision

**A, and A.**

- **`opalro start` and `opalro import` write `feature conflicts`** into a new project's `format`, beside `feature chunks`.
- **`opalro setup --upgrade`** turns on, in an existing project, every feature this version of Opalro knows that the project doesn't list yet (in 0.2: `conflicts`). It says what each one allows and what it costs ("Opalro 0.1 can't open the project afterwards"), asks (`--yes` answers; without a terminal and without `--yes` it changes nothing), then rewrites `.opalro/format` under the project's lock, by the crash-safe write every other file gets, and reads it back. It can't be undone and says so before asking: the diary doesn't hold the format. With nothing to turn on, it says so.
- **`opalro setup`, shown without options, says when an upgrade is there to take,** as decision 008's "it suggests upgrades" asks.
- **A combine that would keep a conflict in a project without the feature stops** as a combine did in milestone 8: it names the files, changes nothing, and says that `opalro setup --upgrade` lets the project keep conflicts, and what that costs. Everything else in v0.2 works in such a project: lines, tags, combines without conflicts.
- **Without the feature, this version treats a conflict entry or record as v0.1 does** (the frozen behaviour in `docs/storage-format.md` §8: it is never decoded, write-out leaves its place alone, `tidy` stops, `verify` reports the listing). Such a thing can only have been planted. With the feature, a conflict is what decisions 174 to 179 say.
- **The `format` file is always written from the features the project has.** Nothing that writes it may drop a line: not the first save, not the step that adds `chunks` to a project from before milestone 5.
- **A feature is never taken away again.**

## Consequences

- The owner's rule holds without an exception: a project gains `conflicts` only when its user asks for it through `opalro setup`.
- Someone who used v0.1 meets one extra command the first time a combine has conflicts, with the reason and the cost in the same message.
- A project made with v0.2 can't be opened by v0.1, from its first save. v0.1 says the project is newer and asks to be updated; it changes nothing.
- A project made by v0.1 and never upgraded stays openable by v0.1, with lines, tags and combined saves in it, as decision 157 promised.
- `opalro setup --upgrade` is the door every later feature comes through too.
- Asking on the spot (option B) would save a step; it is on the rulings' list for the owner, who set the rule.
