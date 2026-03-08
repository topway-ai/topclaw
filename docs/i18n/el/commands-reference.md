# Αναφορά Εντολών TopClaw (CLI Reference)

Αυτός ο οδηγός περιλαμβάνει το πλήρες σύνολο των εντολών που είναι διαθέσιμες στη διεπαφή γραμμής εντολών (CLI) του TopClaw.

Τελευταία ενημέρωση: 8 Μαρτίου 2026.

## Σύνοψη Εντολών

| Εντολή | Περιγραφή |
|:---|:---|
| `onboard` | Εκκίνηση της διαδικασίας αρχικής διαμόρφωσης και εγγραφής. |
| `agent` | Έναρξη αλληλεπίδρασης με τον πράκτορα AI (Interactive Mode). |
| `daemon` | Εκτέλεση του TopClaw ως διεργασία παρασκηνίου (Background Process). |
| `service` | Διαχείριση της υπηρεσίας συστήματος (System Service). |
| `doctor` | Εκτέλεση διαγνωστικών ελέγχων ακεραιότητας και συνδεσιμότητας. |
| `status` | Προβολή της τρέχουσας κατάστασης και των ενεργών ρυθμίσεων. |
| `update` | Έλεγχος ή εγκατάσταση της νεότερης έκδοσης του TopClaw. |
| `backup` | Δημιουργία ή επαναφορά φορητού αντιγράφου πλήρους κατάστασης του TopClaw. |
| `cron` | Διαχείριση προγραμματισμένων εργασιών και αυτοματισμών. |
| `models` | Συγχρονισμός και διαχείριση διαθέσιμων μοντέλων AI. |
| `providers` | Διαχείριση των παρόχων υπολογιστικής ισχύος (LLM Providers). |
| `channel` | Διαμόρφωση και έλεγχος των καναλιών επικοινωνίας. |
| `skills` | Διαχείριση των επεκτάσεων και δυνατοτήτων (Skills) του πράκτορα. |
| `hardware` | Ανίχνευση και διαχείριση συνδεδεμένου υλικού (USB/Serial). |

Συνηθισμένα aliases:

- `topclaw init` -> `topclaw onboard`
- `topclaw chat` -> `topclaw agent`
- `topclaw run` -> `topclaw daemon`
- `topclaw info` -> `topclaw status`
- `topclaw channels` -> `topclaw channel`
- `topclaw skill` -> `topclaw skills`

---

## Ανάλυση Κύριων Εντολών

### 1. `onboard` (Αρχική Διαμόρφωση)

- `topclaw onboard --interactive`: Διαδραστική καθοδήγηση για τη ρύθμιση του συστήματος.
- `topclaw onboard --channels-only`: Εστιασμένη διαμόρφωση αποκλειστικά για τα κανάλια επικοινωνίας.

### 2. `agent` (Διαδραστική Λειτουργία)

- `topclaw agent`: Έναρξη τυπικής συνομιλίας.
- `topclaw agent -m "<μήνυμα>"`: Άμεση αποστολή εντολής/μηνύματος στον πράκτορα.

> [!TIP]
> Κατά τη διάρκεια της συνομιλίας, μπορείτε να αιτηθείτε την αλλαγή του μοντέλου (π.χ. "use gpt-4") και ο πράκτορας θα προσαρμόσει τις ρυθμίσεις του δυναμικά.

### 2.1 `gateway` / `daemon`

- `topclaw gateway [--host <HOST>] [--port <PORT>] [--new-pairing]`
- `topclaw daemon [--host <HOST>] [--port <PORT>]`
- Το `--new-pairing` καθαρίζει όλα τα αποθηκευμένα paired tokens και δημιουργεί νέο pairing code κατά την εκκίνηση του gateway.

### 3. `cron` (Προγραμματισμός Εργασιών)

Δυνατότητα αυτοματισμού εντολών:
- `topclaw cron add "0 9 * * *" "echo Daily Setup"`: Εκτέλεση καθημερινά στις 09:00.
- `topclaw cron once "1h" "topclaw status"`: Προγραμματισμός εκτέλεσης μετά από μία ώρα.

### 4. `doctor` (Διάγνωση Συστήματος)

Χρησιμοποιήστε την εντολή `topclaw doctor` για την επαλήθευση της ορθής λειτουργίας των εξαρτήσεων, της πρόσβασης στο διαδίκτυο και της εγκυρότητας του αρχείου ρυθμίσεων.

Το `topclaw doctor` εμφανίζει πλέον και συγκεκριμένες εντολές επόμενου βήματος όταν εντοπίζει διορθώσιμα προβλήματα ρύθμισης, όπως έλλειψη provider, έλλειψη authentication, μη ρυθμισμένα channels ή απουσία φακέλου workspace.

### 4.1 `status` (Κατάσταση)

- `topclaw status`

Το `topclaw status` εμφανίζει τη συνοπτική εικόνα του config/runtime και πλέον προτείνει επίσης εντολές επόμενου βήματος για σημαντικά κενά ρύθμισης, χρησιμοποιώντας την ίδια λογική προτάσεων με το `topclaw doctor`.

### 5. `update` (Ασφαλής Αναβάθμιση)

- `topclaw update`
- `topclaw update --check`
- `topclaw update --force`

Σημειώσεις:

- Το `topclaw update` κατεβάζει το νεότερο επίσημο release από το GitHub για την τρέχουσα πλατφόρμα και αντικαθιστά το τρέχον binary.
- Το `--check` ελέγχει μόνο αν υπάρχει νέα έκδοση.
- Το `--force` επανεγκαθιστά την τελευταία έκδοση ακόμη και αν η τρέχουσα είναι ήδη η πιο πρόσφατη.
- Αν το TopClaw εκτελείται ως background service, μετά την αναβάθμιση εκτελέστε `topclaw service restart`.
- Αν η θέση του binary δεν είναι εγγράψιμη, το TopClaw εμφανίζει προτεινόμενη διαδρομή ανάκτησης. Σε Linux, η προτεινόμενη λύση είναι:

```bash
curl -fsSL https://raw.githubusercontent.com/jackfly8/TopClaw/main/scripts/install-release.sh | bash
```

### 5.1 `backup` (Αντίγραφο Ασφαλείας / Επαναφορά)

- `topclaw backup create <destination_dir>`
- `topclaw backup create <destination_dir> --include-logs`
- `topclaw backup inspect <source_dir>`
- `topclaw backup restore <source_dir>`
- `topclaw backup restore <source_dir> --force`

Σημειώσεις:

- Το `backup create` εξάγει ολόκληρο το ενεργό config root του TopClaw, συμπεριλαμβανομένων των `config.toml`, authentication state, secrets, memories, preferences, δεδομένων workspace και εγκατεστημένων skills.
- Το `backup create` καταγράφει πλέον checksum για κάθε αρχείο και γράφει ένα `RESTORE.md` μέσα στο bundle ώστε η μεταφορά σε άλλο μηχάνημα να είναι πιο σαφής.
- Το `backup inspect` επαληθεύει την ακεραιότητα του bundle πριν από restore και εμφανίζει τα καταγεγραμμένα σύνολα αρχείων και bytes.
- Τα runtime logs εξαιρούνται από προεπιλογή ώστε το bundle να παραμένει μικρότερο και πιο εύκολα μεταφέρσιμο. Χρησιμοποιήστε `--include-logs` αν θέλετε να τα συμπεριλάβετε.
- Το `backup restore` καλύπτει τόσο disaster recovery όσο και μεταφορά σε άλλο μηχάνημα. Η επαναφορά γράφει στο τρέχον runtime config location και ανανεώνει το active-workspace marker.
- Το `backup restore` αρνείται να αντικαταστήσει μη κενό target directory χωρίς `--force`.
- Στο `backup restore --force`, το TopClaw μετακινεί πρώτα το προηγούμενο target config σε γειτονικό rollback directory αντί να το διαγράφει άμεσα.
- Αν το TopClaw εκτελείται ως background service, σταματήστε ή επανεκκινήστε την υπηρεσία γύρω από την επαναφορά ώστε το runtime να φορτώσει καθαρά την ανακτημένη κατάσταση.

### 6. `skills` (Επεκτασιμότητα)

- `topclaw skills list`: Προβολή εγκατεστημένων δεξιοτήτων.
- `topclaw skills install <source>`: Εγκατάσταση νέας δεξιότητας από εξωτερική πηγή.

> [!NOTE]
> Το TopClaw εφαρμόζει αυτόματη ανάλυση κώδικα (security scanning) σε κάθε νέα δεξιότητα πριν την ενεργοποίησή της για την αποφυγή εκτέλεσης κακόβουλου λογισμικού.

---

## Βοήθεια και Τεκμηρίωση

Για αναλυτικές πληροφορίες σχετικά με τις παραμέτρους κάθε εντολής, χρησιμοποιήστε το flag `--help`:
`topclaw <command> --help`
(π.χ. `topclaw onboard --help`)
