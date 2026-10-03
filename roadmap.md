
1. tutti i campi colore del file di configurazione devono supportare anche l'alpha (dovrebbe già essere cos', da verificare), quindi il formato passa da #ffffff a #ffffffff, se l'aplha non viene specificata viene automaticamente considerata ff
2. la descrizione del sensore viene attualmente renderizzata con un carattere più piccolo, verificare che la dimensione sia relativa alla dimensione specificata nel file di configurazione e non a costante
3. per ogni sensore deve essere possibile specificare il tipo di bordo oltre agli altri campi relativi al bordo, i tipi di bordo selezionabili sono : full (quello attuale), corners (vengono evidenziati solo i 4 bordi del sensore), underline (solo bordo inferiore),upperline(solo bordo superiore),left (solo sinistra),right(solo destra)
