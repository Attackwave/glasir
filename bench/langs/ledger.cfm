<!--- Keeps the running balance for one account. --->
<cfset limitAmount = 5000>

<cffunction name="refuse" returntype="numeric">
    <cfargument name="owner" type="string">
    <cfset warnOwner(arguments.owner)>
    <cfreturn 0>
</cffunction>

<cffunction name="commitEntry" returntype="numeric">
    <cfargument name="owner" type="string">
    <cfset writeEntry(arguments.owner)>
    <cfreturn 1>
</cffunction>

<!--- Bills the account and returns what is left. --->
<cffunction name="charge" returntype="numeric">
    <cfargument name="amount" type="numeric">
    <cfset refuse(arguments.amount)>
    <cfset commitEntry(arguments.amount)>
    <cfreturn arguments.amount>
</cffunction>
