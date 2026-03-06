# Οδηγός Ενημέρωσης και Απεγκατάστασης στο macOS

Αυτή η σελίδα τεκμηριώνει τις υποστηριζόμενες διαδικασίες ενημέρωσης και απεγκατάστασης του TopClaw στο macOS (OS X).

Τελευταία επαλήθευση: **22 Φεβρουαρίου 2026**.

## 1) Έλεγχος τρέχουσας μεθόδου εγκατάστασης

```bash
which topclaw
topclaw --version
```

Τυπικές τοποθεσίες:

- Homebrew: `/opt/homebrew/bin/topclaw` (Apple Silicon) ή `/usr/local/bin/topclaw` (Intel)
- Cargo/bootstrap/χειροκίνητη: `~/.cargo/bin/topclaw`

Αν υπάρχουν και οι δύο, η σειρά `PATH` του shell σας καθορίζει ποια εκτελείται.

## 2) Ενημέρωση στο macOS

### Α) Εγκατάσταση μέσω Homebrew

```bash
brew update
brew upgrade topclaw
topclaw --version
```

### Β) Εγκατάσταση μέσω Clone + bootstrap

Από τον τοπικό κλώνο του αποθετηρίου:

```bash
git pull --ff-only
./bootstrap.sh --prefer-prebuilt
topclaw --version
```

Αν θέλετε ενημέρωση μόνο από πηγαίο κώδικα:

```bash
git pull --ff-only
cargo install --path . --force --locked
topclaw --version
```

### Γ) Χειροκίνητη εγκατάσταση προκατασκευασμένου binary

Επαναλάβετε τη ροή λήψης/εγκατάστασης με το πιο πρόσφατο αρχείο έκδοσης και επαληθεύστε:

```bash
topclaw --version
```

## 3) Απεγκατάσταση στο macOS

### Α) Διακοπή και αφαίρεση υπηρεσίας background πρώτα

Αυτό αποτρέπει τη συνέχεια εκτέλεσης του daemon μετά την αφαίρεση του binary.

```bash
topclaw service stop || true
topclaw service uninstall || true
```

Αντικείμενα υπηρεσίας που αφαιρούνται από την `service uninstall`:

- `~/Library/LaunchAgents/com.topclaw.daemon.plist`

### Β) Αφαίρεση binary ανά μέθοδο εγκατάστασης

Homebrew:

```bash
brew uninstall topclaw
```

Cargo/bootstrap/χειροκίνητη (`~/.cargo/bin/topclaw`):

```bash
cargo uninstall topclaw || true
rm -f ~/.cargo/bin/topclaw
```

### Γ) Προαιρετικά: αφαίρεση τοπικών δεδομένων εκτέλεσης

Εκτελέστε αυτό μόνο αν θέλετε πλήρη εκκαθάριση ρυθμίσεων, προφίλ auth, logs και κατάστασης workspace.

```bash
rm -rf ~/.topclaw
```

## 4) Επαλήθευση ολοκλήρωσης απεγκατάστασης

```bash
command -v topclaw || echo "topclaw binary not found"
pgrep -fl topclaw || echo "No running topclaw process"
```

Αν το `pgrep` εξακολουθεί να βρίσκει διεργασία, σταματήστε την χειροκίνητα και ελέγξτε ξανά:

```bash
pkill -f topclaw
```

## Σχετικά Έγγραφα

- [One-Click Bootstrap](../one-click-bootstrap.md)
- [Αναφορά Εντολών](../commands-reference.md)
- [Αντιμετώπιση Προβλημάτων](../troubleshooting.md)
