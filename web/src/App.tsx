import { useCallback, useEffect, useRef, useState } from "react";
import type { Agent, Archetype, Company, ProjectDetail, Task } from "./lib/api";
import { api } from "./lib/api";
import { useLive } from "./lib/live";
import { useTheme } from "./lib/theme";
import { TopBar } from "./components/TopBar";
import { Board } from "./components/Board";
import { TaskDetail } from "./components/TaskDetail";
import { HireAgentDialog } from "./components/HireAgentDialog";
import { CreateTaskDialog } from "./components/CreateTaskDialog";
import { Onboarding } from "./components/Onboarding";
import { Spinner } from "./components/ui/primitives";

const LAST_COMPANY = "overmind-last-company";

export default function App() {
  const { theme, toggle } = useTheme();

  const [companies, setCompanies] = useState<Company[]>([]);
  const [companyId, setCompanyId] = useState<string | null>(null);
  const [archetypes, setArchetypes] = useState<Archetype[]>([]);

  const [agents, setAgents] = useState<Agent[]>([]);
  const [tasks, setTasks] = useState<Task[]>([]);
  const [projects, setProjects] = useState<ProjectDetail[]>([]);
  const [loading, setLoading] = useState(true);

  const [openTask, setOpenTask] = useState<Task | null>(null);
  const [hireOpen, setHireOpen] = useState(false);
  const [taskOpen, setTaskOpen] = useState(false);
  const [tick, setTick] = useState(0); // bumped on every live change → drives refetch

  // Bootstrap: companies + archetype catalog.
  useEffect(() => {
    Promise.all([api.listCompanies(), api.listArchetypes()])
      .then(([cs, arch]) => {
        setCompanies(cs);
        setArchetypes(arch);
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
    const [a, t, p] = await Promise.all([
      api.listAgents(id),
      api.listTasks(id),
      api.listProjects(id),
    ]);
    setAgents(a);
    setTasks(t);
    setProjects(p);
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
  const { connected } = useLive((changed) => {
    if (changed === null) api.listCompanies().then(setCompanies);
    if (!changed || changed === companyIdRef.current) setTick((n) => n + 1);
  });

  const bump = () => setTick((n) => n + 1);

  const selectedCompany = companies.find((c) => c.id === companyId) ?? null;
  // A runnable goal: the first goal of a project that has a primary workspace.
  const runnableGoalId =
    projects.find((p) => p.workspaces.some((w) => w.is_primary))?.goals[0]?.id ?? null;
  const needsWorkspace = !!companyId && runnableGoalId === null;

  const afterCompanyCreated = async (id: string) => {
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
    <div className="flex h-screen flex-col">
      <TopBar
        companies={companies}
        companyId={companyId}
        onSelectCompany={setCompanyId}
        onNewCompany={() => setCompanyId(null)}
        onHire={() => setHireOpen(true)}
        onNewTask={() => setTaskOpen(true)}
        canCreateTask={runnableGoalId !== null}
        connected={connected}
        tick={tick}
        theme={theme}
        onToggleTheme={toggle}
      />

      {!companyId || needsWorkspace ? (
        <Onboarding
          company={selectedCompany}
          needsWorkspace={needsWorkspace}
          onCompanyCreated={afterCompanyCreated}
          onReady={() => companyId && loadCompany(companyId)}
        />
      ) : (
        <main className="flex flex-1 flex-col overflow-hidden pt-4">
          <Board tasks={tasks} agents={agents} onOpenTask={setOpenTask} />
        </main>
      )}

      <TaskDetail
        task={openTask}
        agents={agents}
        tick={tick}
        onClose={() => setOpenTask(null)}
        onChanged={bump}
      />

      {companyId && (
        <>
          <HireAgentDialog
            open={hireOpen}
            onOpenChange={setHireOpen}
            companyId={companyId}
            archetypes={archetypes}
            onHired={bump}
          />
          <CreateTaskDialog
            open={taskOpen}
            onOpenChange={setTaskOpen}
            companyId={companyId}
            goalId={runnableGoalId}
            onCreated={bump}
          />
        </>
      )}
    </div>
  );
}
