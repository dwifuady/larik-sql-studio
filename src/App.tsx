import { AppLayout } from './components/AppLayout';
import { CreateSpaceModal } from './components/CreateSpaceModal';
import { ErrorBoundary } from './components/ErrorBoundary';
import { McpApprovalDialog } from './components/McpApprovalDialog';
import { UpdateBanner } from './components/UpdateBanner';
import { useAppStore } from './store';

function App() {
  const { createSpaceModalOpen, setCreateSpaceModalOpen } = useAppStore();

  return (
    <>
      <ErrorBoundary>
        <AppLayout />
      </ErrorBoundary>
      <UpdateBanner />
      <McpApprovalDialog />
      <CreateSpaceModal
        isOpen={createSpaceModalOpen}
        onClose={() => setCreateSpaceModalOpen(false)}
      />
    </>
  );
}

export default App;
