import { useCallback, useEffect, useRef, useState } from "react";
import type {
  Agent,
  AgentBudget,
  Archetype,
  Domain,
  Model,
  Company,
  LanguageCode,
  Notification,
  OrgProposal,
  ProjectDetail,
  Task,
  View,
  Economy,
  PlanWindow,
} from "./lib/api";
import { api } from "./lib/api";
import { useLive } from "./lib/live";
import { LanguageProvider } from "./components/LanguageProvider";
import { browserLanguage } from "./lib/i18n";
import { useTheme } from "./lib/theme";
import { TopBar } from "./components/TopBar";
import { Board } from "./components/Board";
import { Chat } from "./components/Chat";
import { Meetings } from "./components/Meetings";
import { Memory } from "./components/Memory";
import { OrgChart } from "./components/OrgChart";
import { TaskDetail } from "./components/TaskDetail";
import { Toaster } from "./components/Toaster";
import { HireAgentDialog } from "./components/HireAgentDialog";
import { CreateTaskDialog } from "./components/CreateTaskDialog";
import { ConnectRepoDialog } from "./components/ConnectRepoDialog";
import { Onboarding } from "./components/Onboarding";
import { Door } from "./components/Door";
import { InviteDialog } from "./components/InviteDialog";
import { DeleteCompanyDialog } from "./components/DeleteCompanyDialog";
import { MembersDialog } from "./components/MembersDialog";
import { SignInNotice } from "./components/SignInNotice";
import { Spinner } from "./components/ui/primitives";

const LAST_COMPANY = "overmind-last-company";

export default function App() {
  const { theme, toggle } = useTheme();

  const [companies, setCompanies] = useState<Company[]>([]);
  const [companyId, setCompanyId] = useState<string | null>(null);
  const [archetypes, setArchetypes] = useState<Archetype[]>([]);
  const [domains, setDomains] = useState<Domain[]>([]);
  const [models, setModels] = useState<Model[]>([]);

  const [agents, setAgents] = useState<Agent[]>([]);
  const [tasks, setTasks] = useState<Task[]>([]);
  const [projects, setProjects] = useState<ProjectDetail[]>([]);
  const [budgets, setBudgets] = useState<AgentBudget[]>([]);
  const [proposals, setProposals] = useState<OrgProposal[]>([]);
  /**
   * How the server pays (ADR-0030). A property of the machine, not of a
   * company, so it is fetched once beside the catalogs rather than on every
   * company switch. `null` until it answers, and the interface simply says
   * nothing about the meaning of a cap until then — better than flashing the
   * wrong meaning and correcting it.
   */
  const [economy, setEconomy] = useState<Economy | null>(null);
  /** The door (M24): nothing behind it is fetched before a session exists. */
  const [gate, setGate] = useState<"checking" | "unclaimed" | "locked" | "in">("checking");
  /** Who is signed in (M25): the invite surface is the owner's. */
  const [isOwner, setIsOwner] = useState(false);
  const [inviteOpen, setInviteOpen] = useState(false);
  const [deleteCompanyOpen, setDeleteCompanyOpen] = useState(false);
  const [membersOpen, setMembersOpen] = useState(false);
  /** The signed-in name (M25): the members list says "you" beside it. */
  const [meName, setMeName] = useState<string | null>(null);
  /** Where each plan window stands; refreshed on every live change. */
  const [planWindows, setPlanWindows] = useState<Record<string, PlanWindow>>({});
  const [loading, setLoading] = useState(true);

  const [view, setView] = useState<View>("chat");
  const [openTask, setOpenTask] = useState<Task | null>(null);
  const [hireOpen, setHireOpen] = useState(false);
  const [hireManager, setHireManager] = useState<string | null>(null);
  const [taskOpen, setTaskOpen] = useState(false);
  const [repoOpen, setRepoOpen] = useState(false);
  const [tick, setTick] = useState(0); // bumped on every live change → drives refetch

  // Notifications: toasts as they arrive, and a signal that opens the inbox.
  const [toasts, setToasts] = useState<Notification[]>([]);
  const [inboxSignal, setInboxSignal] = useState(0);
  const [selectedMeeting, setSelectedMeeting] = useState<string | null>(null);

  const openMeeting = (id: string) => {
    setSelectedMeeting(id);
    setView("meetings");
  };

  const openHire = (managerId: string | null = null) => {
    setHireManager(managerId);
    setHireOpen(true);
  };

  // A session that expires mid-use sends you back to the door instead of
  // leaving a dead app: the API client announces every 401.
  useEffect(() => {
    const onUnauthorized = () => setGate("locked");
    window.addEventListener("overmind:unauthorized", onUnauthorized);
    return () => window.removeEventListener("overmind:unauthorized", onUnauthorized);
  }, []);

  // The door first, everything else after: the boot fetches run only once a
  // session exists (or no owner does).
  useEffect(() => {
    api
      .authState()
      .then((a) => {
        setGate(a.state === "in" ? "in" : a.state);
        setIsOwner(a.state === "in" && a.role === "owner");
        setMeName(a.state === "in" ? (a.name ?? null) : null);
      })
      .catch(() => setGate("in")); // an unreachable server shows its own errors
  }, [gate === "in"]);

  // Bootstrap: companies + both catalogs + the models we ship (ADR-0021).
  useEffect(() => {
    if (gate !== "in") return;
    Promise.all([
      api.listCompanies(),
      api.listArchetypes(),
      api.listDomains(),
      api.listModels(),
    ])
      .then(([cs, arch, doms, mods]) => {
        setCompanies(cs);
        setArchetypes(arch);
        setDomains(doms);
        setModels(mods);
        const last = localStorage.getItem(LAST_COMPANY);
        setCompanyId(cs.find((c) => c.id === last)?.id ?? cs[0]?.id ?? null);
      })
      .finally(() => setLoading(false));
  }, [gate]);

  useEffect(() => {
    if (companyId) localStorage.setItem(LAST_COMPANY, companyId);
  }, [companyId]);

  /**
   * How we pay, and where the plan stands. The economy is fixed for the life of
   * the server; the plan window is not — it is learned from each run, so it is
   * refetched on every live change alongside the rest.
   */
  const refreshHealth = useCallback(() => {
    // Not knowing how we pay must never stop the app; the interface has words
    // for `unknown` and none for a failed boot.
    api
      .health()
      .then((h) => {
        setEconomy(h.economy);
        setPlanWindows(h.plan_windows);
      })
      .catch(() => setEconomy(null));
  }, []);

  useEffect(() => {
    refreshHealth();
  }, [refreshHealth, tick]);

  // Load everything for the selected company.
  const loadCompany = useCallback(async (id: string) => {
    const [a, t, p, b, op] = await Promise.all([
      api.listAgents(id),
      api.listTasks(id),
      api.listProjects(id),
      api.budgetSummary(id),
      api.listOrgProposals(id).catch(() => []),
    ]);
    setAgents(a);
    setTasks(t);
    setProjects(p);
    setBudgets(b);
    setProposals(op);
  }, []);

  useEffect(() => {
    if (companyId) loadCompany(companyId);
  }, [companyId, loadCompany, tick]);

  // Keep the open task's data in sync with refetched tasks.
  const openTaskId = openTask?.id;
  useEffect(() => {
    if (openTaskId) setOpenTask(tasks.find((t) => t.id === openTaskId) ?? null);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [tasks]);

  // Live updates: refetch companies list + current board on any change.
  const companyIdRef = useRef(companyId);
  companyIdRef.current = companyId;
  const { connected } = useLive(
    (changed) => {
      if (changed === null) api.listCompanies().then(setCompanies);
      if (!changed || changed === companyIdRef.current) setTick((n) => n + 1);
    },
    // Something the company wants to tell you right now. Keep a few on screen;
    // the inbox is the record, so dropping older toasts loses nothing.
    (changed, notification) => {
      if (changed && changed !== companyIdRef.current) return;
      setToasts((current) =>
        [
          // A later word on the same subject retires the earlier one: once a
          // meeting is decided, "waiting on you" is a lie on screen.
          ...current.filter((t) => t.id !== notification.id && !sameSubject(t, notification)),
          notification,
        ].slice(-4),
      );
      setTick((n) => n + 1);
    },
  );

  /** Two notifications about the same thing (e.g. one meeting). */
  const sameSubject = (a: Notification, b: Notification) =>
    !!a.subject_id && a.subject_type === b.subject_type && a.subject_id === b.subject_id;

  /** A decision taken in the UI: drop the toast that was asking for it. */
  const afterDecision = (approvalId?: string) => {
    if (approvalId) setToasts((current) => current.filter((t) => t.approval_id !== approvalId));
    setTick((n) => n + 1);
  };

  const bump = () => setTick((n) => n + 1);

  // The language belongs to the company (M16): switching company switches
  // language, because it is that organization's working language. Before the
  // first company exists there is nothing to read it off, so the setup screens
  // follow the browser.
  const language: LanguageCode =
    companies.find((c) => c.id === companyId)?.language ?? browserLanguage();
  useEffect(() => {
    document.documentElement.lang = language;
  }, [language]);
  const changeLanguage = async (code: LanguageCode) => {
    if (!companyId) return;
    await api.setCompanyLanguage(companyId, code);
    setCompanies(await api.listCompanies());
  };

  // A runnable goal: the first goal of a project that has a primary workspace.
  // Only `code` tasks need one — a company with no repo is fully usable for
  // knowledge work (ADR-0017), so this gates the Code option, never the app.
  const runnableGoalId =
    projects.find((p) => p.workspaces.some((w) => w.is_primary))?.goals[0]?.id ?? null;
  const hasRepo = runnableGoalId !== null;

  const afterCompanyCreated = async (id: string) => {
    // The language was chosen on the first screen and set when the company was
    // created. This used to patch it afterwards with `browserLanguage()`, in a
    // second request whose failure was swallowed — so the founding CEO could
    // answer in a language nobody picked, and did.
    const cs = await api.listCompanies();
    setCompanies(cs);
    setCompanyId(id);
  };

  // The mirror of the above (ADR-0034): the server already forgot the
  // company, so the list is refetched and the selection moves to whatever
  // remains — or to onboarding, which is what `null` renders.
  const afterCompanyDeleted = async () => {
    const cs = await api.listCompanies();
    setCompanies(cs);
    setCompanyId(cs[0]?.id ?? null);
    if (cs.length === 0) localStorage.removeItem(LAST_COMPANY);
  };

  if (gate === "checking") {
    return (
      <div className="flex h-screen items-center justify-center">
        <Spinner className="h-6 w-6 text-muted-foreground" />
      </div>
    );
  }
  if (gate === "unclaimed" || gate === "locked") {
    return (
      <LanguageProvider language={language}>
        <div className="flex h-screen flex-col">
          <Door mode={gate} onEntered={() => setGate("in")} />
        </div>
      </LanguageProvider>
    );
  }
  if (loading) {
    return (
      <div className="flex h-screen items-center justify-center">
        <Spinner className="h-6 w-6 text-muted-foreground" />
      </div>
    );
  }

  return (
    <LanguageProvider language={language}>
      <div className="flex h-screen flex-col">
        <TopBar
          companies={companies}
          companyId={companyId}
          onSelectCompany={setCompanyId}
          onNewCompany={() => setCompanyId(null)}
          onHire={() => openHire(null)}
          onNewTask={() => setTaskOpen(true)}
          canCreateTask={!!companyId}
          view={view}
          onViewChange={setView}
          showViews={!!companyId}
          onApprovalDecided={afterDecision}
          inboxSignal={inboxSignal}
          onOpenMeeting={openMeeting}
          connected={connected}
          tick={tick}
          language={language}
          onChangeLanguage={changeLanguage}
          theme={theme}
          onToggleTheme={toggle}
          onLogout={() => {
            api.authLogout().finally(() => setGate("locked"));
          }}
          onInvite={isOwner ? () => setInviteOpen(true) : undefined}
          onDeleteCompany={companyId ? () => setDeleteCompanyOpen(true) : undefined}
          onMembers={companyId ? () => setMembersOpen(true) : undefined}
        />
        <InviteDialog open={inviteOpen} onOpenChange={setInviteOpen} />
        {companyId && (
          <MembersDialog
            open={membersOpen}
            onOpenChange={setMembersOpen}
            companyId={companyId}
            me={meName}
          />
        )}
        {companyId && (
          <DeleteCompanyDialog
            open={deleteCompanyOpen}
            onOpenChange={setDeleteCompanyOpen}
            companyId={companyId}
            companyName={companies.find((c) => c.id === companyId)?.name ?? ""}
            onDeleted={afterCompanyDeleted}
          />
        )}

        <SignInNotice economy={economy} onSignedIn={refreshHealth} />
        {!companyId ? (
          <Onboarding defaultLanguage={language} onDone={afterCompanyCreated} />
        ) : (
          <main className="flex flex-1 flex-col overflow-hidden pt-4">
            {view === "chat" ? (
              <Chat companyId={companyId} agents={agents} tick={tick} onChanged={bump} />
            ) : view === "board" ? (
              <Board
                tasks={tasks}
                agents={agents}
                onOpenTask={setOpenTask}
                hasRepo={hasRepo}
                onConnectRepo={() => setRepoOpen(true)}
              />
            ) : view === "meetings" ? (
              <Meetings
                companyId={companyId}
                tick={tick}
                onChanged={bump}
                selectedId={selectedMeeting}
                onSelect={setSelectedMeeting}
              />
            ) : view === "memory" ? (
              <Memory companyId={companyId} tick={tick} />
            ) : (
              <OrgChart
                agents={agents}
                budgets={budgets}
                economy={economy}
                planWindows={planWindows}
                proposal={proposals.find((p) => p.status === "proposed") ?? null}
                onChanged={bump}
                onHireUnder={openHire}
                onTalkToCeo={() => setView("chat")}
              />
            )}
          </main>
        )}

        <TaskDetail
          task={openTask}
          agents={agents}
          tick={tick}
          onClose={() => setOpenTask(null)}
          onChanged={bump}
        />

        <Toaster
          toasts={toasts}
          onDismiss={(id) => setToasts((current) => current.filter((t) => t.id !== id))}
          onOpen={() => {
            setToasts([]);
            setInboxSignal((n) => n + 1);
          }}
        />

        {companyId && (
          <>
            <HireAgentDialog
              open={hireOpen}
              onOpenChange={setHireOpen}
              companyId={companyId}
              archetypes={archetypes}
              domains={domains}
              models={models}
              agents={agents}
              defaultManager={hireManager}
              onHired={bump}
            />
            <CreateTaskDialog
              open={taskOpen}
              onOpenChange={setTaskOpen}
              companyId={companyId}
              goalId={runnableGoalId}
              onCreated={bump}
              onConnectRepo={() => {
                setTaskOpen(false);
                setRepoOpen(true);
              }}
            />
            <ConnectRepoDialog
              open={repoOpen}
              onOpenChange={setRepoOpen}
              companyId={companyId}
              onConnected={bump}
            />
          </>
        )}
      </div>
    </LanguageProvider>
  );
}
