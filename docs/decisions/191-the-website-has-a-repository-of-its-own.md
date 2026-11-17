# 191 — The website has a repository of its own

- **Status:** Accepted
- **Date:** 2026-11-16
- **Agenda topic:** none (the website, which the owner asked for once v0.2 was packaged)
- **Decided by:** the owner ("B, create it"), on Claude's recommendation. Point 2 of the decision is Claude's addition; the owner can change it.
- **Refines:** [decision 144](144-where-people-get-bolt-and-report-problems.md) (the owner's download page is a website built from a repository of its own) and [decision 145](145-one-release-workflow-for-every-version.md) (the copies of the website's documents are what a release hands that repository)

## Context

**In plain words.** Every build sends people to https://opalro.com ([decision 151](151-opalro-com-and-support-at-opalro-com.md)). On 2026-11-16 nothing stood there: the name pointed at a parking address. With v0.2 packaged and waiting to be published, the owner asked for the website itself to be built, in place of putting files up by hand: a download that works like the Git website's, one place for every document about Opalro, and room for a hosted home for Opalro projects later. The first question was where the website's code lives.

This repository is the workshop where the program is made. The website is the shop window. Three things about the workshop bear on the answer:

- **Its pace.** Every push here, except to a `gate/` branch, starts the tests on Windows, macOS and Linux, and the Windows job alone takes 45 to 65 minutes. Once the repository is private ([decision 140](140-bolt-is-closed-source.md)) those minutes come out of a monthly allowance.
- **Its rules.** `main` takes a version only by the owner's hand ([decision 157](157-v0.2-in-four-milestones.md)), and a milestone is merged only after its red-team session ([decision 024](024-red-team-gate.md)). A website has to be fixable on any day.
- **Its documents.** They are those of the version being built. The website has to show those of the version that is published.

The hand-over between the two already exists. Each release makes `opalro-<version>-website.zip`: the eleven documents meant for the website, with no link into the repository, and the workflow fails when a copy still holds the repository's address ([decision 145](145-one-release-workflow-for-every-version.md), `.github/release/website.sh`).

## Options

- **A. A folder of this repository.** One place, with the documents beside the site. But a typo fixed on the site would start the three-system tests and wait behind the rules above. The site would be built from the documents of a version not yet published, unless it were pinned to a release. And whoever is given access to the site, a hosting service or a designer, would be given the engine's source, which is closed.
- **B. A repository of its own, on the owner's account.** The site takes each release's zip and its four archives, shows exactly the published version, and is fixed and put up on its own days. It costs one copy step per release and a second repository to keep.
- **C. A GitHub organization that holds both.** Addresses like a company's, and access given per repository when other people join. It is more to set up, with a plan of its own, for no gain while one person works on Opalro. A repository moves into an organization later with its history.

## Decision

**B.** The website's code lives in a private repository of its own, `opalro-website`, on the owner's account. This repository stays the engine's alone.

1. **What crosses over.** For each release: the website zip, the four archives and `SHA256SUMS`, all from that one release. Nothing else: the website publishes no file taken from this repository's folders.
2. **Where the documents are written.** Here. The website shows them and doesn't reword them. A wrong sentence is corrected here, and the next release carries the correction. One that can't wait for a release is the owner's call.
3. **Where the downloads are.** With the website. Decision 144 already keeps every page from pointing into this repository, and a private repository's release page gives nothing to anyone without access to it. The release stays the owner's staging place.
4. **What the website's repository decides for itself,** and records there: where the site is hosted, what it is built with, where the downloads are stored, and how it looks.
5. **Going live is the owner's own step:** pointing opalro.com at the site, like publishing a release.
6. **The hosting product** that comes later, a home for Opalro projects as GitHub is for Git, isn't placed by this decision. Its server will use the engine, so that part may belong here; its pages won't.

## Consequences

- From 2026-11-16 there are two repositories. The new one starts with no site in it: its rules, the brief the design was made from, and a note of where the design is.
- The owner's page for v0.2 still lists what goes on the website (part 10). How it gets there becomes the website repository's business once the site is built.
- The release workflow rehearses a website: a folder laid out with the zip's documents and the downloads, served on the runner. Once the real site exists, its layout and the rehearsal's have to agree. Changing the rehearsal is a change here, reviewed like any other.
- Two repositories can drift apart: the site could offer one version's downloads beside another version's documents. So the website takes both from one release in one step, and its download page carries that release's `SHA256SUMS` (risk 22 in [design.md](../design.md)).
- Moving both into an organization (option C) stays open for the day a second person joins or the hosting product starts.
