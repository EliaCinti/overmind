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

  // Bootstrap: companies + both catalogs + the models we ship (ADR-0021).
  useEffect(() => {
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
  }, []);

  useEffect(() => {
    if (companyId) localStorage.setItem(LAST_COMPANY, companyId);
  }, [companyId]);

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
    // A new company keeps speaking whatever the setup screens spoke.
    await api.setCompanyLanguage(id, browserLanguage()).catch(() => {});
    const cs = await api.listCompanies();
    setCompanies(cs);
    setCompanyId(id);
  };

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
        />

        {!companyId ? (
          <Onboarding onDone={afterCompanyCreated} />
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
