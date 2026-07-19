# Overmind — Regole di sessione

Overmind è l'orchestratore open-source di team di agenti AI di Elia: modello aziendale alla Paperclip + execution layer alla Vibe Kanban + memoria organizzativa via MCP. Wadachi è il cervello first-party (managed brain per organizzazione, ADR-0004) ma resta un progetto SEPARATO e SEMPRE opzionale. Stack: Rust (axum + SQLite) backend, React + TypeScript frontend. Licenza MIT.

## Regola zero: Wadachi

A inizio sessione chiama `get_context` con questa directory (progetto registrato: `overmind`). Durante il lavoro usa `recall` prima di investigare e `store_memory`/`store_decision` quando scopri o decidi qualcosa.

## Regole di lavoro

1. **Documentation-driven.** Prima di scrivere codice, leggi `docs/ROADMAP.md`: si lavora SOLO sul milestone `in-progress`, un milestone alla volta, e si chiude solo quando i criteri di accettazione passano. Non iniziare M(N+1) con M(N) a metà.
2. **Ogni decisione architetturale è un ADR** in `docs/adr/` (usa il template 0000). Se `ARCHITECTURE.md` e un ADR sono in disaccordo, vince l'ADR finché ARCHITECTURE.md non viene aggiornato.
3. **Licenze:** Paperclip è MIT (si può portare codice con attribuzione). Vibe Kanban: verificare la licenza prima di portare codice. **Aperant è AGPL-3.0: idee sì, codice MAI.**
4. **Qualità:** niente `unwrap()` in codice non-test, `cargo fmt` + `clippy` puliti, test per gli invarianti (atomicità checkout+budget, append-only dell'audit log, hash chain).
5. **Indipendenza da Wadachi:** ogni feature che tocca la memoria deve funzionare anche senza provider configurato — testare sempre entrambi i casi. MAI vendorare codice Wadachi in questo repo (integrazione = MCP + process management); i brain gestiti da Overmind vivono in `<data-dir>/orgs/<org>/brain/` — MAI toccare il brain personale di Elia (`/Volumes/ExtremeSSD/wadachi-brain`). Modifiche necessarie a Wadachi (es. accesso concorrente multi-agente) si sviluppano nel repo Wadachi.
6. **UX:** ogni feature visibile all'utente segue `docs/UX.md` — progressive disclosure (pick → tune → expert), click-first, opzioni strutturate mappate su config enforced dal server (mai solo prompt text). La caratterizzazione degli agenti è structured-first (ADR-0005).
7. Commit e push solo su richiesta o conferma di Elia. Documenti e codice in inglese; con Elia si parla in italiano.
