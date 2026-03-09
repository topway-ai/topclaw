# Εγχειρίδιο Heartbeat

Χρησιμοποιήστε αυτό το έγγραφο όταν θέλετε το heartbeat του daemon στο TopClaw να λειτουργεί σαν επίμονη περιοδική παρακολούθηση και όχι σαν τυφλό επαναλαμβανόμενο timer.

## Τι άλλαξε

Τα heartbeat tasks έχουν πλέον κατάσταση:

- οι απλές γραμμές `- task` συνεχίζουν να λειτουργούν
- κάθε task αποθηκεύει ιστορικό στο `state/heartbeat_state.json`
- κάθε task έχει cooldown αντί να εκτελείται σε κάθε tick
- επαναλαμβανόμενες αποτυχίες μπαίνουν σε backoff
- tasks με `max_runs=1` σταματούν αφού εκτελεστούν αρκετές φορές
- κάθε tick εκτελεί μόνο τα due tasks με την υψηλότερη προτεραιότητα

## Σύνταξη HEARTBEAT.md

Βασικό task:

```md
- Review my calendar
```

Task με metadata:

```md
- [every=4h] [priority=2] Review my calendar for the next 24 hours
- [every=1d] Check active repos for stale branches
- [every=30m] [max_runs=1] Remind me to finish onboarding notes
```

Υποστηριζόμενα metadata:

- `every=<duration>` ή `cooldown=<duration>`
- `priority=<integer>`
- `max_runs=<integer>`

Παραδείγματα διάρκειας:

- `30m`
- `4h`
- `1d`

## Συμπεριφορά λειτουργίας

- Αν το `HEARTBEAT.md` έχει tasks, το TopClaw προγραμματίζει αυτά.
- Αν δεν υπάρχουν bullet tasks, γίνεται fallback στο `heartbeat.message` όταν έχει οριστεί.
- Νέα tasks είναι άμεσα due.
- Επιτυχημένα runs προγραμματίζουν νέο `next_due_at` από το cooldown.
- Αποτυχημένα runs ξαναδοκιμάζονται νωρίτερα στην αρχή και μετά μπαίνουν σε backoff.
- Tasks που αφαιρούνται από το αρχείο παύουν να επιλέγονται· παλαιότερες state εγγραφές μπορεί να παραμείνουν ως ιστορικό.

## Σχετικά αρχεία

- Πηγή tasks: `<workspace>/HEARTBEAT.md`
- Κατάσταση tasks: `<workspace>/state/heartbeat_state.json`
- Snapshot υγείας daemon: `~/.topclaw/daemon_state.json`

## Προτεινόμενη πρακτική

- Κρατήστε τα heartbeat tasks μικρά και συγκεκριμένα.
- Προτιμήστε 3-10 σταθερά tasks αντί για τεράστιες wish lists.
- Χρησιμοποιήστε `priority=2` ή μεγαλύτερο μόνο όταν κάτι πρέπει πραγματικά να προηγείται.
- Χρησιμοποιήστε `max_runs=1` για one-off υπενθυμίσεις.
- Μην βάζετε καταστροφικές shell εντολές απευθείας στα heartbeat prompts.

## Γρήγορος έλεγχος

1. Εκκινήστε τον daemon.
2. Προσθέστε ένα δοκιμαστικό task στο `HEARTBEAT.md`.
3. Περιμένετε ένα heartbeat interval.
4. Επιβεβαιώστε ότι το `state/heartbeat_state.json` περιέχει `last_run_at`, `next_due_at` και counters.
5. Επιβεβαιώστε ότι το ίδιο task δεν ξανατρέχει αμέσως αν δεν είναι ακόμη due.

## Rollback

Αν το νέο scheduling δεν είναι αυτό που θέλετε:

1. Σταματήστε τον daemon.
2. Απλοποιήστε το `HEARTBEAT.md` ξανά σε απλές γραμμές `- task`.
3. Διαγράψτε το `<workspace>/state/heartbeat_state.json` αν θέλετε πλήρες reset της μνήμης heartbeat.
4. Επανεκκινήστε τον daemon.
