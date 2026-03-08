# Προετοιμασία με Ένα Κλικ (One-Click Bootstrap)

Αυτός ο οδηγός περιγράφει την ταχύτερη μέθοδο για την εγκατάσταση και την αρχικοποίηση του TopClaw.

Τελευταία επαλήθευση: **7 Μαρτίου 2026**.

## Ασφαλής Ενημέρωση

Για υπάρχουσες εγκαταστάσεις, η απλούστερη υποστηριζόμενη διαδρομή ενημέρωσης είναι:

```bash
topclaw update
topclaw --version
```

Για έλεγχο χωρίς εγκατάσταση:

```bash
topclaw update --check
```

Αν το TopClaw εκτελείται ως background service, επανεκκινήστε την υπηρεσία μετά την ενημέρωση:

```bash
topclaw service restart
```

Αν το `topclaw update` αναφέρει ότι δεν μπορεί να αντικαταστήσει το τρέχον binary, επιστρέψτε στη μέθοδο αρχικής εγκατάστασης:

- εγκατάσταση από checkout αποθετηρίου: `./bootstrap.sh --prefer-prebuilt`
- εγκατάσταση από source: `cargo install --path . --force --locked`
- εγκατάσταση από package manager: ενημέρωση μέσω του αντίστοιχου package manager

## Επιλογή 0: Homebrew (macOS / Linux)

```bash
brew install topclaw
```

## Επιλογή Α: Τοπικό Σενάριο (Προτεινόμενο)

1. **Κλωνοποίηση του αποθετηρίου**:
   ```bash
   git clone https://github.com/jackfly8/TopClaw.git
   cd TopClaw
   ```
2. **Εκτέλεση του bootstrap**:
   ```bash
   ./bootstrap.sh
   ```

### Λειτουργία Σενάριου

Από προεπιλογή, το σενάριο εκτελεί:
1. `cargo build --release --locked`
2. `cargo install --path . --force --locked`

### Απαιτήσεις Πόρων και Προ-μεταγλωττισμένα Αρχεία

Η μεταγλώττιση από τον πηγαίο κώδικα απαιτεί τουλάχιστον **2GB RAM** και **6GB ελεύθερο χώρο** στον δίσκο. Σε περίπτωση περιορισμένων πόρων, μπορείτε να χρησιμοποιήσετε προ-μεταγλωττισμένα (prebuilt) αρχεία:

- **Χρήση προ-μεταγλωττισμένων (εάν υπάρχουν)**:
  ```bash
  ./bootstrap.sh --prefer-prebuilt
  ```
- **Αποκλειστική χρήση προ-μεταγλωττισμένων**:
  ```bash
  ./bootstrap.sh --prebuilt-only
  ```
- **Επιβολή μεταγλώττισης από πηγαίο κώδικα**:
  ```bash
  ./bootstrap.sh --force-source-build
  ```

## Προετοιμασία Περιβάλλοντος (Dual-mode)

Για νέα συστήματα που δεν διαθέτουν το σύνολο εργαλείων Rust, χρησιμοποιήστε τις παρακάτω σημαίες:
```bash
./bootstrap.sh --install-system-deps --install-rust
```
- `--install-system-deps`: Εγκαθιστά τις απαραίτητες εξαρτήσεις συστήματος (ενδέχεται να απαιτεί `sudo`).
- `--install-rust`: Εγκαθιστά τη Rust μέσω του `rustup`.

## Επιλογή Β: Απομακρυσμένη Εκτέλεση

```bash
curl -fsSL https://raw.githubusercontent.com/jackfly8/TopClaw/main/scripts/bootstrap.sh | bash
```
> [!IMPORTANT]
> Για περιβάλλοντα υψηλής ασφάλειας, συνιστάται η **Επιλογή Α**, ώστε να μπορείτε να επιθεωρήσετε το σενάριο πριν από την εκτέλεση.

## Διαδικασία Εισαγωγής (Onboarding)

### Μέσω Docker / Podman

```bash
./bootstrap.sh --docker
```
Το σενάριο θα δημιουργήσει μια τοπική εικόνα Docker και θα ξεκινήσει τη διαδικασία onboarding. Οι ρυθμίσεις αποθηκεύονται στον κατάλογο `./.topclaw-docker`.

### Μη Διαδραστική Εισαγωγή

```bash
./bootstrap.sh --onboard --api-key "sk-..." --provider openrouter
```

### Διαδραστική Εισαγωγή

```bash
./bootstrap.sh --interactive-onboard
```

## Αναφορά Σημαιών CLI

- `--install-system-deps`: Εγκατάσταση εξαρτήσεων συστήματος.
- `--install-rust`: Εγκατάσταση του Rust toolchain.
- `--skip-build`: Παράλειψη της διαδικασίας μεταγλώττισης.
- `--skip-install`: Παράλειψη της εγκατάστασης του εκτελέσιμου.
- `--provider <id>`: Ορισμός παρόχου LLM.

Για την πλήρη λίστα επιλογών, εκτελέστε:
```bash
./bootstrap.sh --help
```

## Σχετική Τεκμηρίωση

- [README.md](../README.md)
- [commands-reference.md](commands-reference.md)
- [providers-reference.md](providers-reference.md)
- [channels-reference.md](channels-reference.md)
