import { createContext, useContext } from "react";
import type { LanguageCode, Notification } from "./api";

/**
 * The UI dictionary (M16, slice C).
 *
 * No library: this is a few hundred strings and a lookup, and a dependency
 * would buy plural rules and lazy bundles we do not need yet. What it must buy
 * is safety — `t()` is typed against the English dictionary, so a key that does
 * not exist, or a translation that drifts out of the shape, fails the build
 * rather than rendering `undefined` at someone.
 *
 * The language itself is *not* stored here: it belongs to the company, because
 * the server needs it to tell agents what to write (see `i18n.rs`). This module
 * only renders it.
 *
 * Section names of enum-shaped tables (`status`, `priority`, `autonomy`) match
 * the server's values exactly, so `t(`status.${task.status}`)` type-checks and
 * a new status added server-side fails the build until it is translated.
 */
export const en = {
  nav: {
    chat: "Chat",
    board: "Board",
    meetings: "Meetings",
    org: "Org",
    hire: "Hire",
    newTask: "New task",
    newCompany: "+ New company…",
    noCompany: "No company",
    language: "Language",
    toggleTheme: "Toggle theme",
    inbox: "Inbox",
    unread: "{n} unread",
    waitingOnYou: "{n} waiting on your decision",
    nothingWaiting: "Nothing waiting on you.",
    liveConnected: "Live updates connected",
    reconnecting: "Reconnecting…",
    memory: "memory",
    /** The view's tab. Separate from the badge above, which is deliberately
     *  lowercase next to its icon — a tab beside Chat/Board/Org is not. */
    memoryView: "Memory",
    memoryOn: "Organizational memory connected (Wadachi)",
    memoryOff: "Memory not configured",
    audit: "audit",
    auditOk: "Audit chain verified",
    auditBroken: "Audit chain BROKEN",
    deleteCompany: "Delete company",
    deleteCompanyWarning:
      "This deletes {name} — its agents, tasks, conversations, meetings and memory. The audit history stays. There is no undo.",
    deleteCompanyType: "Type the company's name to confirm",
    deleteCompanyConfirm: "Delete forever",
    deleteCompanyBusy: "A session is still running — wait for it to finish, then try again.",
  },
  common: {
    approve: "Approve",
    reject: "Reject",
    decline: "Decline",
    cancel: "Cancel",
    back: "Back",
    dismiss: "Dismiss",
    working: "Working…",
    failed: "Something went wrong",
    connectRepo: "Connect a repo",
    markAllRead: "Mark all read",
    viewMeeting: "View meeting",
  },
  status: {
    backlog: "Backlog",
    todo: "To do",
    in_progress: "In progress",
    in_review: "In review",
    blocked: "Blocked",
    done: "Done",
    cancelled: "Cancelled",
  },
  priority: {
    low: "Low",
    medium: "Medium",
    high: "High",
    urgent: "Urgent",
  },
  autonomy: {
    propose_only: "Propose only",
    act_with_approval: "Act with approval",
    act_within_budget: "Act within budget",
  },
  /** What each autonomy level means, as a sentence, for the hire preview. */
  autonomySays: {
    propose_only: "proposes changes but never acts without you",
    act_with_approval: "acts on tasks once you approve each start",
    act_within_budget: "picks up and runs tasks on its own, within budget",
  },
  strictness: {
    lenient: "Lenient",
    standard: "Standard",
    strict: "Strict",
  },
  // The two axes of characterization (ADR-0021). Keyed by the server's own
  // slugs, like `status` and `autonomy`, so a catalog row added server-side
  // fails the build until every language names it. A slug that is *not* here is
  // a user's own row — `useCatalogText` shows its stored prose rather than
  // inventing a translation.
  archetype: {
    "chief-executive": "Chief Executive",
    builder: "Builder",
    reviewer: "Reviewer",
    researcher: "Researcher",
    writer: "Writer",
    analyst: "Analyst",
  },
  archetypeDesc: {
    "chief-executive":
      "Runs the company. Turns what you want into an organization and a plan, delegates rather than executing, and escalates the calls that are yours.",
    builder: "Builds the thing itself: implements, assembles, configures. Hands changes over for review rather than putting them live.",
    reviewer:
      "Judges work against a standard — correctness, quality, safety — and says what is wrong and why. Reads everything, changes nothing.",
    researcher:
      "Investigates open questions, compares the options honestly, and writes up what it found with its sources.",
    writer: "Turns what the company knows into something a person can read: guides, references, briefs.",
    analyst: "Works the numbers: costs, projections, comparisons. Shows the model it used, not only the answer.",
  },
  domain: {
    general: "General",
    software: "Software",
    backend: "Backend",
    frontend: "Frontend",
    security: "Security",
    "media-av": "Media & A/V",
    "home-systems": "Home & Building Systems",
    finance: "Finance",
    legal: "Legal & Compliance",
  },
  domainDesc: {
    general: "No particular field. Pick this when the work is not about one subject in particular.",
    software: "Software as a whole: source, architecture, and the tests that hold it up.",
    backend: "The server side: APIs, data models, business logic.",
    frontend: "The interface people actually touch: components, styling, accessibility.",
    security: "Vulnerabilities, secrets handling, dependency risk, and who is allowed to do what.",
    "media-av": "Picture and sound: display and projection, audio reproduction, calibration, acoustics.",
    "home-systems": "Physical spaces and what gets installed in them: layout, wiring, mounting, standards.",
    finance: "Money: costs, projections, unit economics, and the risk hiding in both.",
    legal: "Contracts, licensing, compliance — and knowing when a qualified human must sign off.",
  },
  board: {
    noRepo:
      "No git repo connected. Agents can research, write documents and decide — connect a repo when you want them writing code.",
    emptyColumn: "Nothing here",
  },
  chat: {
    talkingTo: "You're talking to",
    talkTo: "Talk to",
    selectAgent: "Select agent",
    emptyTitle: "Talk to {name}",
    emptyTeam: "your team",
    emptyLeader:
      "Describe what you want — a decision, a piece of research, a change to ship. The CEO breaks it down, opens the right tasks, and puts the team on it.",
    emptyTeammate:
      "Ask {name} directly. They reply in their role, open tasks, and loop in teammates (or the CEO) when your request affects them.",
    thisAgent: "this agent",
    placeholder: "Message {name}…",
    placeholderNoAgents: "Hire an agent to talk to first…",
    theAgent: "the agent",
    attach: "Attach files",
    send: "Send message",
    remove: "Remove {name}",
    hintLeader: "The CEO decomposes what you ask into tasks and dispatches the team.",
    hintTeammate: "{name} can act in their role and pull in teammates when it affects them.",
    unreachable: "Could not reach the agent.",
    agent: "Agent",
    escalation: "Escalation",
  },
  meetings: {
    emptyTitle: "No meetings yet",
    emptyBody:
      "Agents ask for one when they hit a call none of them should make alone. You approve it first — nothing runs before that.",
    asked: "{name} asked",
    youConvened: "You convened",
    select: "Select a meeting",
    why: "Why: ",
    cap: "cap {n}",
    waiting: "{name} is waiting on you. Nothing has run yet.",
    anAgent: "An agent",
    noTurns: "No turns yet.",
    deliberating: "deliberating…",
    decision: "Decision",
    carried: "Everyone in the room carries this into their work",
    approvedBy: "Approved by {name}",
    declinedBy: "Declined by {name}",
    resume: "Resume",
    pausedFallback: "An agent ran out of budget. Raise its cap, then resume.",
  },
  memory: {
    memories: "Memories",
    decisions: "Decisions",
    search: "Search what the company knows…",
    fromTask: "from task",
    fromMeeting: "from meeting",
    noSubject: "No recorded source",
    // The four reasons a page can be empty are four different problems.
    emptyTitle: "Nothing remembered yet",
    emptyBody:
      "A finished task leaves a memory behind, and it shows up here with the work that produced it.",
    // Said separately, because "nothing remembered yet" on this tab would be
    // false whenever the company has memories and simply no decisions.
    emptyDecisionsTitle: "No decisions yet",
    emptyDecisionsBody:
      "When a meeting reaches a call, the decision is recorded here with the room that made it.",
    noResultsTitle: "Nothing matches",
    noResultsBody: "Try fewer words — this searches by meaning, not by exact text.",
    noProviderTitle: "Memory is not set up",
    noProviderBody:
      "Point OVERMIND_MEMORY_CMD at an MCP memory server and the company starts remembering its work.",
    brainOffTitle: "This company's brain is off",
    brainOffBody:
      "Its agents work exactly as before, they just stop remembering. Switch it back on to resume.",
    notBrowsableTitle: "This provider can't be browsed",
    notBrowsableBody:
      "It stores and returns memory, but does not answer with a list we can read. The memory loop itself is unaffected.",
  },
  sessionStatus: {
    running: "running",
    completed: "completed",
    failed: "failed",
  },
  meetingStatus: {
    requested: "Waiting on you",
    open: "In session",
    decided: "Decided",
    declined: "Declined",
    failed: "Failed",
    paused: "Out of budget",
  },
  inbox: {
    empty:
      "Nothing yet. When an agent needs you — to convene a meeting, to start a gated task — it lands here.",
    toastWaiting: "Waiting on you — click to decide",
    approvedBy: "Approved by {name}",
    rejectedBy: "Rejected by {name}",
  },
  /**
   * Notifications. The server sends the parts (`params`), we word them — see
   * `useNotificationText`. Agent-authored prose inside the params is printed
   * as it came: the agent already wrote it in this language.
   */
  notif: {
    meetingRequestedTitle: "{agent} wants to convene a meeting",
    meetingRequestedBody:
      "Topic: {topic}\n\nWhy: {reason}\n\nIn the room: {roster}\n\nUp to {turnCap} turns, then whoever chairs it must call the decision. Nothing runs until you approve.",
    meetingDecidedTitle: "Decided: {topic}",
    meetingDeclinedTitle: "Meeting declined",
    meetingDeclinedBody: 'You declined the meeting on "{topic}". It will not run.',
    meetingDeclinedBodyNote: 'You declined the meeting on "{topic}": {note}',
    meetingDroppedTitle: "No decision needed: {topic}",
    meetingDroppedBody: "{agent} closed the room: {why}",
    meetingFailedTitle: "Meeting could not run: {topic}",
    orgProposedTitle: "{agent} proposes a team",
    orgProposedBody:
      "{agent} has drawn up a team of {count}:\n\n{roster}\n\nWhy: {summary}\n\nNobody is hired until you accept. You can drop anyone from the list first.",
    orgRejectedTitle: "Team proposal declined",
    orgRejectedBody: "You declined the proposed team.",
    orgRejectedBodyNote: "You declined the proposed team: {note}",
    approvalRequestedTitle: "{agent} wants to start a task",
    approvalRequestedBody: "Task: {task}\n\nThis agent is gated: it starts only once you approve.",
    budgetExhaustedTitle: "{agent} is out of budget",
    budgetExhaustedBody:
      "{spent} of {limit} spent this month, so {agent} could not take its turn. Raise its cap, or wait for the new month.",
    meetingPausedTitle: "Meeting paused: {topic}",
    meetingPausedPlanTitle: "Meeting paused: {topic}",
    meetingPausedPlanBody:
      "The subscription has run out for the {window}. Nothing is over budget — {agent} can carry on once it resets {when}.",
    meetingPausedBody:
      "{agent} reached its monthly budget ({spent} of {limit}), so the room stopped mid-discussion. Nothing is lost — raise the cap or wait for the new month, then resume it.",
  },
  door: {
    claimTitle: "Claim this Overmind",
    claimBody: "No owner exists yet. Choose a name and a password: whoever claims it first owns it.",
    loginTitle: "Welcome back",
    loginBody: "This Overmind is locked. Sign in to continue.",
    name: "Name",
    password: "Password",
    passwordHint: "At least 8 characters.",
    claim: "Claim and enter",
    login: "Enter",
    failed: "That is not the right name and password.",
    limited: "Too many attempts. Wait a minute.",
    logout: "Sign out",
    welcome: "Welcome",
    chooseBody: "Your AI company runs here. Sign in, or create your account.",
    chooseLogin: "Log in",
    chooseSignup: "Sign up",
    back: "Back",
    signupTitle: "Create your account",
    signupBody: "Name and password live only in this Overmind's own database, hashed, on this machine.",
    signupFirstHint: "No account exists yet: the first one created owns this Overmind.",
    signup: "Create and enter",
    signupTaken: "That name cannot be used.",
    whereLogin: "Log in",
    whereSignup: "Sign up",
    inviteCode: "Invite code",
    inviteHint: "Ask whoever runs this Overmind for one.",
    inviteMint: "Invite someone",
    inviteMinted: "One-time code, valid 7 days. It is shown only now:",
    inviteCopy: "Copy",
    members: "Members",
    membersHint:
      "Anyone in the company can bring in a colleague who already has an account here. Accounts come from the owner's invites.",
    membersAdd: "Add",
    membersAddPlaceholder: "Their name on this Overmind",
    membersUnknown: "Nobody by that name has an account here.",
    membersSince: "since {date}",
    membersOwner: "owner",
    membersYou: "you",
    inviteCopied: "Copied.",
  },
  economy: {
    key: "Billed to an API key",
    keyMeaning: "The cap is a ceiling in real money.",
    keyOverridesLogin: "You are signed in with a subscription, and it is not paying.",
    keyOverridesLoginFix:
      "An API key takes precedence over a claude.ai login. Overmind can let the plan pay instead — the offer is at the top of the page.",
    payerPlanChosen: "You chose the plan; the key is kept out of the agents' environment.",
    letPlanPay: "Let the plan pay",
    letPlanPayBody:
      "Overmind can keep ANTHROPIC_API_KEY out of the agents' environment and ask the CLI again who pays. Nothing else changes; the choice survives a restart and can be undone from the org chart.",
    letPlanPayWorking: "Asking the CLI who pays…",
    letPlanPayDone: "Done. The plan pays from now on.",
    letPlanPayDoneWithPlan: "Done. The {plan} plan pays from now on.",
    letPlanPayFailed: "Overmind could not make the plan pay:",
    keepKey: "Keep the key",
    backToKey: "Go back to whatever is detected",
    subscription: "Covered by a subscription",
    subscriptionWithPlan: "Covered by a {plan} subscription",
    subscriptionMeaning:
      "Amounts are equivalents, not charges. Of the plan itself we can see the window, not how much of it is left.",
    unknown: "Overmind cannot tell how it is paying",
    unknownMeaning: "The cap still brakes a looping agent, but do not read it as a promise.",
    notSignedIn: "The agents cannot pay yet",
    notSignedInBody:
      "The agent CLI is not signed in, so a conversation would fail instead of answering. Give it one of these, then restart Overmind:",
    notSignedInKey: "Pay with an API key: export it where Overmind starts",
    notSignedInPlan: "Pay with a Claude subscription: sign in once, a volume keeps it",
    notSignedInPlanHost: "Running from source instead? Sign in with: claude login",
    connectPlan: "Connect your Claude subscription",
    connectPlanStarting: "Starting the sign-in…",
    connectPlanOpen: "1. Open this link and authorize:",
    connectPlanPaste: "2. Paste the code you receive:",
    connectPlanSubmit: "Connect",
    connectPlanExchanging: "Checking the code…",
    connectPlanDone: "Signed in. Your subscription is paying from now on.",
    connectPlanFailed: "The sign-in did not complete. The CLI said:",
    connectPlanRetry: "Try again",
    left: "{pct}% left",
    ofCap: "{used} of {cap}",
    approxOfCap: "≈{used} of {cap}",
    nextRun: "next: ≈{task} a task · ≈{turn} a turn",
    nextRunFrom:
      "Priced from this agent's own ledger — {task} task runs, {turn} turns on record. Fewer than three and the flat default stands.",
    spentNoCap: "{used} spent",
    usedThisMonth: "Overmind has used {amount} this month.",
    monthNotPlanWindow:
      "Counted over a calendar month. Your plan measures its own windows, which Overmind cannot see from here.",
    windowFiveHour: "5-hour window",
    windowSevenDay: "7-day window",
    windowOther: "plan window",
    windowResets: "resets {when}",
    planWarning: "close to the limit",
    planExhausted: "the plan has run out for this window",
    planAllowed: "fine",
    windowUnreported: "not reported yet",
    planLifeline: "Your plan",
  },
  org: {
    you: "You",
    ownerLine: "Owner · everyone ultimately reports here",
    empty: "No agents yet. Hire your first one to build the org.",
    twoRoadsTitle: "{ceo} is your CEO, and the company is otherwise empty.",
    twoRoadsBody:
      "Tell {ceo} what you want to build and it will design the team — who to hire, in what role, reporting to whom — and put the chart in front of you before anyone is hired.",
    tellTheIdea: "Tell {ceo} the idea",
    orBuildYourself: "or build the team yourself",
    paused: "paused",
    approvalBadge: "approval",
    edit: "Edit",
    hireReport: "Hire a report",
    title: "Title",
    titlePlaceholder: "e.g. Senior Engineer",
    reportsTo: "Reports to",
    youOwner: "You (owner)",
    governance: "Governance",
    pause: "Pause",
    resume: "Resume",
    requireApproval: "Require approval",
    dropApproval: "Drop approval gate",
    terminate: "Terminate",
    terminateConfirm: "Terminate {name}? This is permanent.",
    saveEdits: "Save title / manager",
  },
  proposal: {
    heading: "{ceo} proposes a team of {n}",
    hires: "Approving hires {n}. Nobody has been hired yet.",
    hiresAndSkips: "Approving hires {n} and skips {skipped}. Nobody has been hired yet.",
    allDropped: "You dropped everyone — put someone back, or decline the proposal.",
    hire: "Hire {n}",
    hiring: "Hiring…",
    skipped: "Skipped. Anyone reporting to them will report to the CEO instead.",
    skipOne: "Skip {name}",
    putBack: "Put {name} back",
  },
  task: {
    priorityLabel: "{p} priority",
    actions: "Actions",
    startWithAgent: "Start with agent",
    moveTo: "Move to {status}",
    terminal: "Terminal state — no moves.",
    noActiveAgents: "No active agents — hire one first.",
    latestRun: "Latest run",
    viewDiff: "View diff",
    rawOutput: "Adapter output",
    noChanges: "No changes in the worktree.",
    loadingDocs: "Loading documents…",
    noDocs: "No documents produced yet.",
    runs: "{n} runs on this task.",
    actionFailed: "Action failed",
    deliverables: "Delivered",
    download: "Download",
    downloadNamed: "Download {name}",
    expand: "Show",
    collapse: "Hide",
    inputs: "Files for this task",
    inputsHint:
      "Whatever the agent should read: documents, data, images, code. It gets them in its working directory.",
    attach: "Attach a file",
    attaching: "Uploading…",
    detach: "Remove {name}",
    noInputs: "Nothing attached.",
    attachFailed: "Could not attach the file",
  },
  hire: {
    title: "Hire an agent",
    pickDesc: "Pick a role to start — everything is preconfigured.",
    pickFunctionDesc: "Pick what kind of work this agent does.",
    pickDomainDesc: "Now the field it works in — it adds focus and capabilities on top.",
    domainStep: "Field",
    fieldOf: "{function} · pick a field",
    multimodal: "Works with images",
    multimodalHint:
      "Looks at pictures, screenshots and diagrams. Tasks that carry them can only go to an agent that does.",
    previewLooks: " It looks at visual material.",
    tune: "tune the details",
    expert: "expert mode",
    name: "Name",
    jobTitle: "Title",
    jobTitleHint: "Optional job title.",
    jobTitlePlaceholder: "e.g. Senior Engineer",
    reportsTo: "Reports to",
    reportsToHint: "Where this agent sits in the org.",
    youOwner: "You (owner)",
    focus: "Focus areas",
    focusHint: "What this agent pays attention to.",
    autonomy: "Autonomy",
    strictness: "Review strictness",
    budget: "Monthly budget · {amount}",
    model: "Model",
    tools: "Tools",
    toolsHint:
      "MCP servers the operator declared on this machine. Granted per agent and written into the run's own configuration — an agent holds exactly what you grant here, nothing it finds.",
    previewTools: " Holds the tools: {tools}.",
    brief: "Custom brief",
    briefHint:
      "Added on top of the structured config. It can add guidance but never override the enforced limits above.",
    briefPlaceholder:
      "e.g. Pay special attention to our authentication module and flag any use of deprecated crypto.",
    expertMode: "Expert mode",
    submit: "Hire agent",
    submitting: "Hiring…",
    failed: "Failed to hire",
    previewReviewing: ", reviewing with ",
    previewStrictness: " strictness on ",
    previewNoFocus: "no specific focus",
    previewCapped: ". Capped at ",
    previewPerMonth: "/mo on ",
    previewBrief: " Plus your custom brief.",
  },
  newTask: {
    title: "New task",
    desc: "Describe the work. An agent can pick it up once it's in To do.",
    titleField: "Title",
    titlePlaceholder: "e.g. Add a health-check endpoint",
    description: "Description",
    descriptionHint: "What the agent should do, and any constraints.",
    descriptionPlaceholder: "Return 200 with { status: ok } at GET /health…",
    priority: "Priority",
    kind: "Kind",
    kindHint: "Code produces a diff in a git worktree; Knowledge produces documents.",
    code: "Code",
    knowledge: "Knowledge",
    needsRepo: "A code task needs a git repo to branch a worktree from. Knowledge tasks don't.",
    submit: "Create task",
    submitting: "Creating…",
    failed: "Failed to create task",
  },
  repo: {
    title: "Connect a git repo",
    desc: "Agents work here — each code run gets its own isolated worktree.",
    path: "Repository path",
    pathHint: "An absolute path to a git repository on this machine.",
    pathPlaceholder: "/Users/you/code/my-project",
    submit: "Connect",
    submitting: "Connecting…",
    failed: "Failed to connect the repository",
  },
  connections: {
    title: "Connections",
    desc: "Let a Claude Code session, or anything that speaks MCP, file work into this company and read its board. It files; it never starts a run.",
    label: "What is connecting",
    labelHint: "So you can tell one from another when you come to withdraw it.",
    labelPlaceholder: "e.g. my editor",
    create: "Connect",
    onceOnly: "shown once. Paste this into the MCP configuration of whatever is connecting.",
    copy: "Copy configuration",
    copied: "Copied",
    done: "Done",
    revoke: "Withdraw",
    revoked: "withdrawn",
    neverUsed: "never used",
    lastUsed: "last used {when}",
    empty: "Nothing is connected yet.",
  },
  onboard: {
    step1: "Step 1 of 2",
    step2: "Step 2 of 2 · optional",
    nameTitle: "Name your company",
    nameSubtitle: "An organization of AI agents that work for you.",
    companyName: "Company name",
    companyPlaceholder: "e.g. Acme Labs",
    language: "Working language",
    languageHint: "What your agents write in — replies, documents, decisions. Changeable later.",
    continue: "Continue",
    creating: "Creating…",
    repoTitle: "Connect a git repo",
    repoSubtitle:
      "Only for agents that write code — each run gets its own isolated worktree. Research, documents and decisions need no repo.",
    finish: "Finish setup",
    settingUp: "Setting up…",
    skip: "Skip — no code work yet",
  },
} as const;

/** A translation must have exactly the shape of the English one. */
export type Dictionary = {
  [S in keyof typeof en]: { [K in keyof (typeof en)[S]]: string };
};

export const it: Dictionary = {
  nav: {
    chat: "Chat",
    board: "Bacheca",
    meetings: "Riunioni",
    org: "Organico",
    hire: "Assumi",
    newTask: "Nuovo task",
    newCompany: "+ Nuova azienda…",
    noCompany: "Nessuna azienda",
    language: "Lingua",
    toggleTheme: "Cambia tema",
    inbox: "Notifiche",
    unread: "{n} da leggere",
    waitingOnYou: "{n} in attesa di una tua decisione",
    nothingWaiting: "Non c'è nulla in attesa.",
    liveConnected: "Aggiornamenti in tempo reale attivi",
    reconnecting: "Riconnessione…",
    memory: "memoria",
    memoryView: "Memoria",
    memoryOn: "Memoria organizzativa collegata (Wadachi)",
    memoryOff: "Memoria non configurata",
    audit: "audit",
    auditOk: "Catena di audit verificata",
    auditBroken: "Catena di audit COMPROMESSA",
    deleteCompany: "Elimina azienda",
    deleteCompanyWarning:
      "Questo elimina {name} — i suoi agenti, task, conversazioni, riunioni e memoria. La storia di audit resta. Non si può annullare.",
    deleteCompanyType: "Scrivi il nome dell'azienda per confermare",
    deleteCompanyConfirm: "Elimina per sempre",
    deleteCompanyBusy: "Una sessione è ancora in corso — aspetta che finisca e riprova.",
  },
  common: {
    approve: "Approva",
    reject: "Rifiuta",
    decline: "Rifiuta",
    cancel: "Annulla",
    back: "Indietro",
    dismiss: "Chiudi",
    working: "Al lavoro…",
    failed: "Qualcosa è andato storto",
    connectRepo: "Collega un repo",
    markAllRead: "Segna tutto come letto",
    viewMeeting: "Vedi la riunione",
  },
  status: {
    backlog: "Backlog",
    todo: "Da fare",
    in_progress: "In corso",
    in_review: "In revisione",
    blocked: "Bloccato",
    done: "Fatto",
    cancelled: "Annullato",
  },
  priority: {
    low: "Bassa",
    medium: "Media",
    high: "Alta",
    urgent: "Urgente",
  },
  autonomy: {
    propose_only: "Solo proposte",
    act_with_approval: "Agisce con approvazione",
    act_within_budget: "Agisce entro il budget",
  },
  autonomySays: {
    propose_only: "propone modifiche ma non agisce mai senza di te",
    act_with_approval: "lavora sui task una volta che approvi ogni avvio",
    act_within_budget: "prende in carico i task ed esegue da solo, entro il budget",
  },
  strictness: {
    lenient: "Permissiva",
    standard: "Standard",
    strict: "Severa",
  },
  archetype: {
    "chief-executive": "Direzione",
    builder: "Costruttore",
    reviewer: "Revisore",
    researcher: "Ricercatore",
    writer: "Redattore",
    analyst: "Analista",
  },
  archetypeDesc: {
    "chief-executive":
      "Guida l'azienda. Trasforma quel che vuoi in un'organizzazione e un piano, delega invece di eseguire, e ti rimanda le decisioni che sono tue.",
    builder:
      "Costruisce la cosa: implementa, assembla, configura. Consegna le modifiche in revisione invece di metterle in produzione.",
    reviewer:
      "Giudica il lavoro rispetto a uno standard — correttezza, qualità, sicurezza — e dice cosa non va e perché. Legge tutto, non tocca nulla.",
    researcher:
      "Indaga le domande aperte, confronta le opzioni onestamente, e scrive quel che ha trovato con le sue fonti.",
    writer:
      "Trasforma quel che l'azienda sa in qualcosa che una persona può leggere: guide, riferimenti, sintesi.",
    analyst:
      "Lavora i numeri: costi, proiezioni, confronti. Mostra il modello che ha usato, non solo il risultato.",
  },
  domain: {
    general: "Generale",
    software: "Software",
    backend: "Backend",
    frontend: "Frontend",
    security: "Sicurezza",
    "media-av": "Audio & Video",
    "home-systems": "Casa & Impianti",
    finance: "Finanza",
    legal: "Legale & Conformità",
  },
  domainDesc: {
    general: "Nessun campo in particolare. Scegli questo quando il lavoro non è su un argomento specifico.",
    software: "Il software nel suo insieme: sorgente, architettura, e i test che lo reggono.",
    backend: "Il lato server: API, modelli di dati, logica di business.",
    frontend: "L'interfaccia che le persone toccano davvero: componenti, stile, accessibilità.",
    security: "Vulnerabilità, gestione dei segreti, rischio nelle dipendenze, e chi può fare cosa.",
    "media-av": "Immagine e suono: display e proiezione, riproduzione audio, calibrazione, acustica.",
    "home-systems": "Spazi fisici e ciò che ci si installa: disposizione, cablaggi, staffaggio, normative.",
    finance: "I soldi: costi, proiezioni, economia unitaria, e il rischio nascosto in entrambi.",
    legal: "Contratti, licenze, conformità — e sapere quando serve la firma di un umano qualificato.",
  },
  board: {
    noRepo:
      "Nessun repo git collegato. Gli agenti possono fare ricerca, scrivere documenti e decidere — collega un repo quando vuoi che scrivano codice.",
    emptyColumn: "Niente qui",
  },
  chat: {
    talkingTo: "Stai parlando con",
    talkTo: "Parla con",
    selectAgent: "Scegli un agente",
    emptyTitle: "Parla con {name}",
    emptyTeam: "la tua squadra",
    emptyLeader:
      "Descrivi cosa vuoi — una decisione, una ricerca, una modifica da rilasciare. Il CEO la scompone, apre i task giusti e ci mette la squadra.",
    emptyTeammate:
      "Chiedi direttamente a {name}. Risponde nel suo ruolo, apre task e coinvolge i colleghi (o il CEO) quando la tua richiesta li riguarda.",
    thisAgent: "questo agente",
    placeholder: "Scrivi a {name}…",
    placeholderNoAgents: "Assumi prima un agente con cui parlare…",
    theAgent: "l'agente",
    attach: "Allega file",
    send: "Invia messaggio",
    remove: "Rimuovi {name}",
    hintLeader: "Il CEO scompone quello che chiedi in task e li assegna alla squadra.",
    hintTeammate: "{name} può agire nel suo ruolo e coinvolgere i colleghi quando li riguarda.",
    unreachable: "Non è stato possibile raggiungere l'agente.",
    agent: "Agente",
    escalation: "Escalation",
  },
  meetings: {
    emptyTitle: "Nessuna riunione",
    emptyBody:
      "Gli agenti ne chiedono una quando arrivano a una scelta che nessuno di loro dovrebbe fare da solo. Prima la approvi tu — niente parte prima di allora.",
    asked: "{name} l'ha chiesta",
    youConvened: "Convocata da te",
    select: "Scegli una riunione",
    why: "Perché: ",
    cap: "max {n} turni",
    waiting: "{name} sta aspettando te. Non è ancora partito nulla.",
    anAgent: "Un agente",
    noTurns: "Ancora nessun turno.",
    deliberating: "in corso…",
    decision: "Decisione",
    resume: "Riprendi",
    pausedFallback: "Un agente ha finito il budget. Alza il suo tetto, poi riprendi.",
    carried: "Chi era nella stanza porta questa decisione nel proprio lavoro",
    approvedBy: "Approvata da {name}",
    declinedBy: "Rifiutata da {name}",
  },
  memory: {
    memories: "Memorie",
    decisions: "Decisioni",
    search: "Cerca in ciò che l'azienda sa…",
    fromTask: "dal task",
    fromMeeting: "dalla riunione",
    noSubject: "Origine non registrata",
    emptyTitle: "Ancora nessuna memoria",
    emptyBody:
      "Un task concluso lascia una memoria, e compare qui insieme al lavoro che l'ha prodotta.",
    emptyDecisionsTitle: "Ancora nessuna decisione",
    emptyDecisionsBody:
      "Quando una riunione arriva a una scelta, la decisione viene registrata qui insieme alla stanza che l'ha presa.",
    noResultsTitle: "Nessuna corrispondenza",
    noResultsBody: "Prova con meno parole — qui si cerca per significato, non per testo esatto.",
    noProviderTitle: "La memoria non è configurata",
    noProviderBody:
      "Punta OVERMIND_MEMORY_CMD a un server MCP di memoria e l'azienda inizia a ricordare il proprio lavoro.",
    brainOffTitle: "Il cervello di questa azienda è spento",
    brainOffBody:
      "I suoi agenti lavorano esattamente come prima, semplicemente smettono di ricordare. Riaccendilo per riprendere.",
    notBrowsableTitle: "Questo provider non si può sfogliare",
    notBrowsableBody:
      "Salva e restituisce memoria, ma non risponde con una lista che sappiamo leggere. Il ciclo della memoria non ne risente.",
  },
  sessionStatus: {
    running: "in esecuzione",
    completed: "completata",
    failed: "fallita",
  },
  meetingStatus: {
    requested: "In attesa di te",
    open: "In seduta",
    decided: "Decisa",
    declined: "Rifiutata",
    failed: "Fallita",
    paused: "Budget esaurito",
  },
  inbox: {
    empty:
      "Ancora niente. Quando un agente ha bisogno di te — per convocare una riunione, per avviare un task sotto approvazione — arriva qui.",
    toastWaiting: "Aspetta te — clicca per decidere",
    approvedBy: "Approvato da {name}",
    rejectedBy: "Rifiutato da {name}",
  },
  notif: {
    meetingRequestedTitle: "{agent} vuole convocare una riunione",
    meetingRequestedBody:
      "Tema: {topic}\n\nPerché: {reason}\n\nIn riunione: {roster}\n\nMassimo {turnCap} turni, poi chi presiede deve dichiarare la decisione. Niente parte finché non approvi.",
    meetingDecidedTitle: "Deciso: {topic}",
    meetingDeclinedTitle: "Riunione rifiutata",
    meetingDeclinedBody: "Hai rifiutato la riunione su «{topic}». Non si terrà.",
    meetingDeclinedBodyNote: "Hai rifiutato la riunione su «{topic}»: {note}",
    meetingDroppedTitle: "Nessuna decisione necessaria: {topic}",
    meetingDroppedBody: "{agent} ha chiuso la stanza: {why}",
    meetingFailedTitle: "La riunione non è potuta partire: {topic}",
    orgProposedTitle: "{agent} propone una squadra",
    orgProposedBody:
      "{agent} ha messo insieme una squadra di {count}:\n\n{roster}\n\nPerché: {summary}\n\nNessuno viene assunto finché non accetti. Prima puoi togliere chi vuoi dalla lista.",
    orgRejectedTitle: "Proposta di squadra rifiutata",
    orgRejectedBody: "Hai rifiutato la squadra proposta.",
    orgRejectedBodyNote: "Hai rifiutato la squadra proposta: {note}",
    approvalRequestedTitle: "{agent} vuole avviare un task",
    approvalRequestedBody:
      "Task: {task}\n\nQuesto agente è sotto approvazione: parte solo quando dai l'ok.",
    budgetExhaustedTitle: "{agent} ha finito il budget",
    budgetExhaustedBody:
      "{spent} di {limit} spesi questo mese, quindi {agent} non ha potuto fare il suo turno. Alza il suo tetto, o aspetta il mese nuovo.",
    meetingPausedTitle: "Riunione in pausa: {topic}",
    meetingPausedPlanTitle: "Riunione in pausa: {topic}",
    meetingPausedPlanBody:
      "L'abbonamento è esaurito per la {window}. Nessuno ha sforato il budget — {agent} può riprendere quando si ricarica {when}.",
    meetingPausedBody:
      "{agent} ha raggiunto il budget mensile ({spent} di {limit}), quindi la stanza si è fermata a metà discussione. Non è andato perso niente — alza il tetto o aspetta il mese nuovo, poi riprendila.",
  },
  door: {
    claimTitle: "Reclama questo Overmind",
    claimBody: "Non esiste ancora un proprietario. Scegli un nome e una password: chi lo reclama per primo ne è il proprietario.",
    loginTitle: "Bentornato",
    loginBody: "Questo Overmind è chiuso a chiave. Accedi per continuare.",
    name: "Nome",
    password: "Password",
    passwordHint: "Almeno 8 caratteri.",
    claim: "Reclama ed entra",
    login: "Entra",
    failed: "Nome e password non corrispondono.",
    limited: "Troppi tentativi. Aspetta un minuto.",
    logout: "Esci",
    welcome: "Benvenuto",
    chooseBody: "Qui gira la tua azienda di agenti. Accedi, oppure crea il tuo account.",
    chooseLogin: "Accedi",
    chooseSignup: "Registrati",
    back: "Indietro",
    signupTitle: "Crea il tuo account",
    signupBody: "Nome e password vivono solo nel database di questo Overmind, hashati, su questa macchina.",
    signupFirstHint: "Non esiste ancora nessun account: il primo creato è il proprietario di questo Overmind.",
    signup: "Crea ed entra",
    signupTaken: "Questo nome non è utilizzabile.",
    whereLogin: "Accesso",
    whereSignup: "Registrazione",
    inviteCode: "Codice invito",
    inviteHint: "Chiedilo a chi gestisce questo Overmind.",
    inviteMint: "Invita qualcuno",
    inviteMinted: "Codice monouso, valido 7 giorni. Viene mostrato solo adesso:",
    inviteCopy: "Copia",
    members: "Membri",
    membersHint:
      "Chiunque sia nell'azienda può far entrare un collega che ha già un account qui. Gli account nascono dagli inviti del proprietario.",
    membersAdd: "Aggiungi",
    membersAddPlaceholder: "Il suo nome su questo Overmind",
    membersUnknown: "Nessuno con quel nome ha un account qui.",
    membersSince: "dal {date}",
    membersOwner: "proprietario",
    membersYou: "tu",
    inviteCopied: "Copiato.",
  },
  economy: {
    key: "Addebitato su una chiave API",
    keyMeaning: "Il tetto è un limite in denaro vero.",
    keyOverridesLogin: "Hai l'accesso con un abbonamento, e non è lui che sta pagando.",
    keyOverridesLoginFix:
      "Una chiave API ha la precedenza su un login claude.ai. Overmind può far pagare il piano al suo posto: l'offerta è in cima alla pagina.",
    payerPlanChosen: "Hai scelto il piano: la chiave resta fuori dall'ambiente degli agenti.",
    letPlanPay: "Fai pagare il piano",
    letPlanPayBody:
      "Overmind può tenere ANTHROPIC_API_KEY fuori dall'ambiente degli agenti e richiedere alla CLI chi paga. Non cambia altro; la scelta sopravvive a un riavvio e si annulla dall'organico.",
    letPlanPayWorking: "Chiedo alla CLI chi paga…",
    letPlanPayDone: "Fatto. Da ora paga il piano.",
    letPlanPayDoneWithPlan: "Fatto. Da ora paga il piano {plan}.",
    letPlanPayFailed: "Overmind non è riuscito a far pagare il piano:",
    keepKey: "Tieni la chiave",
    backToKey: "Torna a quello che viene rilevato",
    subscription: "Coperto da un abbonamento",
    subscriptionWithPlan: "Coperto da un abbonamento {plan}",
    subscriptionMeaning:
      "Gli importi sono equivalenti, non addebiti. Del piano vediamo la finestra, non quanto ne resta.",
    unknown: "Overmind non riesce a capire come sta pagando",
    unknownMeaning: "Il tetto frena comunque un agente in loop, ma non leggerlo come una promessa.",
    notSignedIn: "Gli agenti non possono ancora pagare",
    notSignedInBody:
      "La CLI degli agenti non è autenticata: una conversazione fallirebbe invece di rispondere. Dalle una di queste credenziali, poi riavvia Overmind:",
    notSignedInKey: "Paga con una chiave API: esportala dove parte Overmind",
    notSignedInPlan: "Paga con un abbonamento Claude: accedi una volta, un volume lo conserva",
    notSignedInPlanHost: "Esegui dai sorgenti? Accedi con: claude login",
    connectPlan: "Collega il tuo abbonamento Claude",
    connectPlanStarting: "Avvio dell'accesso…",
    connectPlanOpen: "1. Apri questo link e autorizza:",
    connectPlanPaste: "2. Incolla qui il codice che ricevi:",
    connectPlanSubmit: "Collega",
    connectPlanExchanging: "Verifica del codice…",
    connectPlanDone: "Accesso riuscito. Da ora paga il tuo abbonamento.",
    connectPlanFailed: "L'accesso non è andato a buon fine. La CLI ha detto:",
    connectPlanRetry: "Riprova",
    left: "{pct}% rimasto",
    ofCap: "{used} di {cap}",
    approxOfCap: "≈{used} di {cap}",
    nextRun: "prossimo: ≈{task} a task · ≈{turn} a turno",
    nextRunFrom:
      "Stimato dal registro di questo agente — {task} run di task, {turn} turni registrati. Sotto i tre resta il default piatto.",
    spentNoCap: "{used} spesi",
    usedThisMonth: "Overmind ha usato {amount} questo mese.",
    monthNotPlanWindow:
      "Contato su un mese di calendario. Il tuo piano misura le proprie finestre, che Overmind da qui non vede.",
    windowFiveHour: "Finestra di 5 ore",
    windowSevenDay: "Finestra di 7 giorni",
    windowOther: "Finestra del piano",
    windowResets: "si ricarica {when}",
    planWarning: "vicino al limite",
    planExhausted: "il piano è esaurito per questa finestra",
    planAllowed: "ok",
    windowUnreported: "non ancora riportata",
    planLifeline: "Il tuo piano",
  },
  org: {
    you: "Tu",
    ownerLine: "Proprietario · tutti rispondono qui, in ultima istanza",
    empty: "Nessun agente. Assumi il primo per costruire l'organico.",
    twoRoadsTitle: "{ceo} è il tuo CEO, e l'azienda per ora è vuota.",
    twoRoadsBody:
      "Racconta a {ceo} cosa vuoi costruire: disegnerà la squadra — chi assumere, con che ruolo, chi risponde a chi — e ti metterà davanti l'organigramma prima che venga assunto qualcuno.",
    tellTheIdea: "Racconta l'idea a {ceo}",
    orBuildYourself: "oppure costruisci la squadra tu",
    paused: "in pausa",
    approvalBadge: "approvazione",
    edit: "Modifica",
    hireReport: "Assumi un sottoposto",
    title: "Ruolo",
    titlePlaceholder: "es. Senior Engineer",
    reportsTo: "Risponde a",
    youOwner: "Te (proprietario)",
    governance: "Governance",
    pause: "Metti in pausa",
    resume: "Riprendi",
    requireApproval: "Richiedi approvazione",
    dropApproval: "Togli l'approvazione",
    terminate: "Licenzia",
    terminateConfirm: "Licenziare {name}? È definitivo.",
    saveEdits: "Salva ruolo / responsabile",
  },
  proposal: {
    heading: "{ceo} propone una squadra di {n}",
    hires: "Approvando ne assumi {n}. Non è ancora stato assunto nessuno.",
    hiresAndSkips:
      "Approvando ne assumi {n} e ne scarti {skipped}. Non è ancora stato assunto nessuno.",
    allDropped: "Li hai scartati tutti — rimettine qualcuno, oppure rifiuta la proposta.",
    hire: "Assumi {n}",
    hiring: "Assunzione…",
    skipped: "Scartato. Chi rispondeva a lui risponderà al CEO.",
    skipOne: "Scarta {name}",
    putBack: "Rimetti {name}",
  },
  task: {
    priorityLabel: "priorità {p}",
    actions: "Azioni",
    startWithAgent: "Avvia con un agente",
    // Not "Sposta in {status}": with "In corso" and "In revisione" that stacks
    // two prepositions — "Sposta in In corso". The arrow says the same thing.
    moveTo: "Sposta → {status}",
    terminal: "Stato finale — nessuno spostamento.",
    noActiveAgents: "Nessun agente attivo — assumine uno prima.",
    latestRun: "Ultima esecuzione",
    viewDiff: "Vedi il diff",
    rawOutput: "Output dell'adattatore",
    noChanges: "Nessuna modifica nel worktree.",
    loadingDocs: "Caricamento documenti…",
    noDocs: "Ancora nessun documento prodotto.",
    runs: "{n} esecuzioni su questo task.",
    actionFailed: "Azione fallita",
    deliverables: "Consegnato",
    download: "Scarica",
    downloadNamed: "Scarica {name}",
    expand: "Mostra",
    collapse: "Nascondi",
    inputs: "File per questo task",
    inputsHint:
      "Tutto ciò che l'agente deve leggere: documenti, dati, immagini, codice. Se li ritrova nella sua cartella di lavoro.",
    attach: "Allega un file",
    attaching: "Caricamento…",
    detach: "Togli {name}",
    noInputs: "Nessun allegato.",
    attachFailed: "Non è stato possibile allegare il file",
  },
  hire: {
    title: "Assumi un agente",
    pickDesc: "Scegli un ruolo per iniziare — è già tutto configurato.",
    pickFunctionDesc: "Scegli che tipo di lavoro fa questo agente.",
    pickDomainDesc: "Ora il campo in cui lo fa — aggiunge focus e capacità sopra al ruolo.",
    domainStep: "Campo",
    fieldOf: "{function} · scegli un campo",
    multimodal: "Lavora con le immagini",
    multimodalHint:
      "Guarda foto, screenshot e diagrammi. I task che ne contengono possono andare solo a un agente che lo fa.",
    previewLooks: " Guarda il materiale visivo.",
    tune: "regola i dettagli",
    expert: "modalità esperto",
    name: "Nome",
    jobTitle: "Ruolo",
    jobTitleHint: "Titolo professionale, facoltativo.",
    jobTitlePlaceholder: "es. Senior Engineer",
    reportsTo: "Risponde a",
    reportsToHint: "Dove sta questo agente nell'organigramma.",
    youOwner: "Te (proprietario)",
    focus: "Aree di attenzione",
    focusHint: "A cosa questo agente presta attenzione.",
    autonomy: "Autonomia",
    strictness: "Severità in revisione",
    budget: "Budget mensile · {amount}",
    model: "Modello",
    tools: "Strumenti",
    toolsHint:
      "Server MCP dichiarati dall'operatore su questa macchina. Concessi per agente e scritti nella configurazione del run — un agente ha esattamente ciò che concedi qui, niente che trova da sé.",
    previewTools: " Ha gli strumenti: {tools}.",
    brief: "Brief personalizzato",
    briefHint:
      "Si aggiunge alla configurazione strutturata. Può dare indicazioni, ma non può mai scavalcare i limiti imposti qui sopra.",
    briefPlaceholder:
      "es. Presta particolare attenzione al modulo di autenticazione e segnala ogni uso di crittografia deprecata.",
    expertMode: "Modalità esperto",
    submit: "Assumi l'agente",
    submitting: "Assunzione…",
    failed: "Assunzione fallita",
    previewReviewing: ", rivede con severità ",
    previewStrictness: " su ",
    previewNoFocus: "nessuna area specifica",
    previewCapped: ". Tetto di ",
    previewPerMonth: "/mese su ",
    previewBrief: " Più il tuo brief personalizzato.",
  },
  newTask: {
    title: "Nuovo task",
    desc: "Descrivi il lavoro. Un agente può prenderlo in carico appena è in Da fare.",
    titleField: "Titolo",
    titlePlaceholder: "es. Aggiungi un endpoint di health-check",
    description: "Descrizione",
    descriptionHint: "Cosa deve fare l'agente, e con quali vincoli.",
    descriptionPlaceholder: "Rispondi 200 con { status: ok } su GET /health…",
    priority: "Priorità",
    kind: "Tipo",
    kindHint: "Codice produce un diff in un worktree git; Conoscenza produce documenti.",
    code: "Codice",
    knowledge: "Conoscenza",
    needsRepo:
      "Un task di codice ha bisogno di un repo git da cui ramificare un worktree. I task di conoscenza no.",
    submit: "Crea il task",
    submitting: "Creazione…",
    failed: "Creazione del task fallita",
  },
  repo: {
    title: "Collega un repo git",
    desc: "Gli agenti lavorano qui — ogni esecuzione di codice ha il suo worktree isolato.",
    path: "Percorso del repository",
    pathHint: "Un percorso assoluto a un repository git su questa macchina.",
    pathPlaceholder: "/Users/tu/code/mio-progetto",
    submit: "Collega",
    submitting: "Collegamento…",
    failed: "Collegamento del repository fallito",
  },
  connections: {
    title: "Connessioni",
    desc: "Permetti a una sessione Claude Code, o a qualsiasi cosa parli MCP, di aprire task in questa azienda e leggerne la board. Deposita lavoro; non ne avvia mai.",
    label: "Che cosa si collega",
    labelHint: "Per distinguerle l'una dall'altra quando verrà il momento di ritirarne una.",
    labelPlaceholder: "es. il mio editor",
    create: "Collega",
    onceOnly:
      "mostrato una volta sola. Incollalo nella configurazione MCP di ciò che si sta collegando.",
    copy: "Copia la configurazione",
    copied: "Copiato",
    done: "Fatto",
    revoke: "Ritira",
    revoked: "ritirata",
    neverUsed: "mai usata",
    lastUsed: "ultimo uso {when}",
    empty: "Non è collegato ancora niente.",
  },
  onboard: {
    step1: "Passo 1 di 2",
    step2: "Passo 2 di 2 · facoltativo",
    nameTitle: "Dai un nome alla tua azienda",
    nameSubtitle: "Un'organizzazione di agenti AI che lavorano per te.",
    companyName: "Nome dell'azienda",
    companyPlaceholder: "es. Acme Labs",
    language: "Lingua di lavoro",
    languageHint: "Quella in cui scrivono i tuoi agenti — risposte, documenti, decisioni. Si cambia anche dopo.",
    continue: "Continua",
    creating: "Creazione…",
    repoTitle: "Collega un repo git",
    repoSubtitle:
      "Serve solo agli agenti che scrivono codice — ogni esecuzione ha il suo worktree isolato. Ricerca, documenti e decisioni non richiedono un repo.",
    finish: "Concludi la configurazione",
    settingUp: "Configurazione…",
    skip: "Salta — per ora niente codice",
  },
};

const DICTIONARIES: Record<LanguageCode, Dictionary> = { en, it };

/** `t("org.tellTheIdea", { ceo: "Aria" })` */
export type Translate = (key: TranslationKey, vars?: Record<string, string | number>) => string;

export type TranslationKey = {
  [S in keyof typeof en]: `${S & string}.${keyof (typeof en)[S] & string}`;
}[keyof typeof en];

/**
 * The languages on offer, each named **in its own language** — a speaker finds
 * "Italiano" faster than "Italian", and a list of endonyms needs no translation
 * of its own. Mirrors `i18n::SUPPORTED` on the server.
 *
 * Deliberately no flags: a flag is a country, and languages are not countries.
 * English is not the United Kingdom, and picking one flag for it would be
 * taking a side in someone's argument.
 */
export const LANGUAGES: { code: LanguageCode; name: string }[] = [
  { code: "en", name: "English" },
  { code: "it", name: "Italiano" },
];

/**
 * The best supported match for what the browser says the reader prefers.
 *
 * Used only where there is no company to read a language off — the first-run
 * screens. Someone whose machine is Italian should not have to finish an
 * English setup before they can switch.
 */
export function browserLanguage(): LanguageCode {
  const tags = navigator.languages?.length ? navigator.languages : [navigator.language];
  for (const tag of tags) {
    const base = tag.toLowerCase().split("-")[0];
    if (base in DICTIONARIES) return base as LanguageCode;
  }
  return "en";
}

export const LanguageContext = createContext<LanguageCode>("en");

export function useLanguage(): LanguageCode {
  return useContext(LanguageContext);
}

/** The translator for the current language. */
export function useT(): Translate {
  const language = useLanguage();
  const dict = DICTIONARIES[language] ?? en;
  return (key, vars) => {
    const [section, name] = key.split(".") as [keyof Dictionary, string];
    const table = dict[section] as Record<string, string> | undefined;
    // Fall back to English rather than to the key: a missing Italian string
    // should read as English, not as `proposal.heading`.
    const template =
      table?.[name] ?? (en[section] as Record<string, string> | undefined)?.[name] ?? key;
    if (!vars) return template;
    return Object.entries(vars).reduce(
      (out, [k, v]) => out.replaceAll(`{${k}}`, String(v)),
      template,
    );
  };
}

/**
 * Catalog prose, worded here (ADR-0021).
 *
 * The archetype and domain catalogs are *data*: seedable, extensible, and
 * carrying English `name` / `description` columns the server hands out as-is.
 * Rendering those columns put raw English inside an interface M16 had made
 * fully Italian. So the same rule as notifications applies — the server sends
 * the identity, the client writes the words.
 *
 * The fallback is the point: a slug we do not know is a row a user or a plugin
 * added, and its own prose is a far better answer than its slug.
 */
export function useCatalogText(): (
  kind: "archetype" | "domain",
  slug: string,
  stored: { name: string; description: string },
) => { name: string; description: string } {
  const language = useLanguage();
  const dict = DICTIONARIES[language] ?? en;
  return (kind, slug, stored) => {
    const descKind = `${kind}Desc` as "archetypeDesc" | "domainDesc";
    const names = dict[kind] as Record<string, string>;
    const descs = dict[descKind] as Record<string, string>;
    const enNames = en[kind] as Record<string, string>;
    const enDescs = en[descKind] as Record<string, string>;
    return {
      name: names?.[slug] ?? enNames?.[slug] ?? stored.name,
      description: descs?.[slug] ?? enDescs?.[slug] ?? stored.description,
    };
  };
}

/**
 * A notification, worded here (M16 slice D).
 *
 * The server stores both a finished English sentence and the values it was made
 * of. We prefer the values; we fall back to the sentence for rows written
 * before the params column existed, and for any kind this client does not know
 * — a client one version behind should show an English notification, never an
 * empty one.
 */
export function useNotificationText(): (n: Notification) => { title: string; body: string } {
  const t = useT();
  const { formatCents, timeUntil } = useFormats();
  return (n) => {
    const p = n.params;
    const fallback = { title: n.title, body: n.body };
    if (!p) return fallback;
    const v = (k: string) => String(p[k] ?? "");
    const has = (k: string) => p[k] !== null && p[k] !== undefined && p[k] !== "";
    switch (n.kind) {
      case "meeting.requested":
        return {
          title: t("notif.meetingRequestedTitle", { agent: v("agent") }),
          body: t("notif.meetingRequestedBody", {
            topic: v("topic"),
            reason: v("reason"),
            roster: v("roster"),
            turnCap: v("turnCap"),
          }),
        };
      case "meeting.decided":
        return {
          title: t("notif.meetingDecidedTitle", { topic: v("topic") }),
          // The room's own words, printed as they came.
          body: v("decision"),
        };
      case "meeting.declined":
        return {
          title: t("notif.meetingDeclinedTitle"),
          body: has("note")
            ? t("notif.meetingDeclinedBodyNote", { topic: v("topic"), note: v("note") })
            : t("notif.meetingDeclinedBody", { topic: v("topic") }),
        };
      case "meeting.dropped":
        return {
          title: t("notif.meetingDroppedTitle", { topic: v("topic") }),
          body: t("notif.meetingDroppedBody", { agent: v("agent"), why: v("why") }),
        };
      case "budget.exhausted":
        return {
          title: t("notif.budgetExhaustedTitle", { agent: v("agent") }),
          body: t("notif.budgetExhaustedBody", {
            agent: v("agent"),
            spent: formatCents(Number(p.spentCents ?? 0)),
            limit: formatCents(Number(p.limitCents ?? 0)),
          }),
        };
      case "meeting.paused":
        return {
          title: t("notif.meetingPausedTitle", { topic: v("topic") }),
          body: t("notif.meetingPausedBody", {
            agent: v("agent"),
            spent: formatCents(Number(p.spentCents ?? 0)),
            limit: formatCents(Number(p.limitCents ?? 0)),
          }),
        };
      case "meeting.pausedPlan": {
        // A limit nobody chose, so the sentence must not imply one that can be
        // raised (ADR-0030). The window is named in the reader's language and
        // the countdown is worded by `Intl`, not by us.
        const w = String(p.window ?? "");
        return {
          title: t("notif.meetingPausedPlanTitle", { topic: v("topic") }),
          body: t("notif.meetingPausedPlanBody", {
            agent: v("agent"),
            window:
              w === "five_hour"
                ? t("economy.windowFiveHour")
                : w === "seven_day"
                  ? t("economy.windowSevenDay")
                  : t("economy.windowOther"),
            when: timeUntil(Number(p.resetsAt ?? 0)),
          }),
        };
      }
      case "meeting.failed":
        return {
          title: t("notif.meetingFailedTitle", { topic: v("topic") }),
          // An adapter error, in whatever language the adapter speaks.
          body: v("reason"),
        };
      case "org.proposed":
        return {
          title: t("notif.orgProposedTitle", { agent: v("agent") }),
          body: t("notif.orgProposedBody", {
            agent: v("agent"),
            count: v("count"),
            roster: v("roster"),
            summary: v("summary"),
          }),
        };
      case "org.rejected":
        return {
          title: t("notif.orgRejectedTitle"),
          body: has("note")
            ? t("notif.orgRejectedBodyNote", { note: v("note") })
            : t("notif.orgRejectedBody"),
        };
      case "approval.requested":
        return {
          title: t("notif.approvalRequestedTitle", { agent: v("agent") }),
          body: t("notif.approvalRequestedBody", { task: v("task") }),
        };
      default:
        return fallback;
    }
  };
}

/** How money and time read in this language. */
export function useFormats() {
  const language = useLanguage();
  const locale = language === "it" ? "it-IT" : "en-US";
  // The product is priced in euro; showing dollars was a leftover of the
  // dollar-shaped `_cents` field name, not a decision.
  const money = new Intl.NumberFormat(locale, { style: "currency", currency: "EUR" });
  // Relative time is worded by the platform, not by our dictionary. `Intl`
  // knows that Italian says "un minuto fa" but "2 minuti fa", and that the
  // threshold phrasings differ per language — a table of "{n}m ago" strings
  // would get that wrong in every language we added.
  const relative = new Intl.RelativeTimeFormat(locale, { numeric: "auto", style: "narrow" });
  return {
    locale,
    formatCents: (cents: number) => money.format(cents / 100),
    timeAgo: (iso: string | null | undefined) => {
      if (!iso) return "—";
      const secs = Math.round((Date.now() - new Date(iso).getTime()) / 1000);
      if (secs < 45) return relative.format(0, "second");
      const mins = Math.round(secs / 60);
      if (mins < 60) return relative.format(-mins, "minute");
      const hours = Math.round(mins / 60);
      if (hours < 24) return relative.format(-hours, "hour");
      return relative.format(-Math.round(hours / 24), "day");
    },
    /**
     * The same idea pointed forward, for a plan window that has not reset yet
     * (ADR-0030). `Intl` words it: "tra 2 ore" and "in 2 hours" differ in more
     * than vocabulary, and a table of "{n}h" strings gets that wrong per
     * language.
     */
    timeUntil: (epochSeconds: number) => {
      const secs = Math.round(epochSeconds - Date.now() / 1000);
      if (secs <= 0) return relative.format(0, "second");
      const mins = Math.round(secs / 60);
      if (mins < 60) return relative.format(mins, "minute");
      const hours = Math.round(mins / 60);
      if (hours < 24) return relative.format(hours, "hour");
      return relative.format(Math.round(hours / 24), "day");
    },
  };
}
