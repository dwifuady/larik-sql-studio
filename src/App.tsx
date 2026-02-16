import { AppLayout } from './components/AppLayout';
import { CreateSpaceModal } from './components/CreateSpaceModal';
import { useAppStore } from './store';

function App() {
  const { createSpaceModalOpen, setCreateSpaceModalOpen } = useAppStore();

  return (
    <>
      <AppLayout />
      <CreateSpaceModal
        isOpen={createSpaceModalOpen}
        onClose={() => setCreateSpaceModalOpen(false)}
      />
    </>
  );
}

export default App;
