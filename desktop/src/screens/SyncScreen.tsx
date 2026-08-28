import { useEffect, useState } from "react";
import { ScreenHeader } from "@/components/ui";
import { AccountCard } from "@/screens/sync/AccountCard";
import { DeviceCard } from "@/screens/sync/DeviceCard";
import { EncryptionCard } from "@/screens/sync/EncryptionCard";
import { PhraseModal } from "@/screens/sync/PhraseModal";
import { RestoreModal } from "@/screens/sync/RestoreModal";
import { ScopeCard } from "@/screens/sync/ScopeCard";
import { SyncHeroCard } from "@/screens/sync/SyncHeroCard";
import { useSyncStore } from "@/stores/sync";

export function SyncScreen() {
  const load = useSyncStore((s) => s.load);
  const [showPhrase, setShowPhrase] = useState(false);
  const [showRestore, setShowRestore] = useState(false);

  useEffect(() => { void load(); }, [load]);

  return (
    <div className="screen vscroll">
      <div className="screen-inner narrow">
        <ScreenHeader eyebrow="CLOUD SYNC" title="云同步" />
        <SyncHeroCard />
        <AccountCard />
        <ScopeCard />
        <DeviceCard />
        <EncryptionCard
          onRestore={() => setShowRestore(true)}
          onShowPhrase={() => setShowPhrase(true)}
        />
      </div>
      {showPhrase && <PhraseModal onClose={() => setShowPhrase(false)} />}
      {showRestore && (
        <RestoreModal
          onClose={() => setShowRestore(false)}
          onDone={() => { setShowRestore(false); void load(); }}
        />
      )}
    </div>
  );
}
