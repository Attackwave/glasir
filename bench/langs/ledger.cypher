// Keeps the running balance for one account.

CREATE (limit:Config {value: 5000})

CREATE (refuse:Action {name: 'refuse'})

CREATE (commitEntry:Action {name: 'commitEntry'})

// Bills the account and returns what is left.
CREATE (charge:Action {name: 'charge'})-[:CALLS]->(refuse)
CREATE (charge)-[:CALLS]->(commitEntry)
