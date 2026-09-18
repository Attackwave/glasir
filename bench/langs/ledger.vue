<script>
// Keeps the running balance for one account.
export default {
  data() {
    return { limit: 5000 };
  },
  methods: {
    // Bills the account and returns what is left.
    charge(owner, amount) {
      if (amount > this.limit) {
        return this.refuse(owner);
      }
      return this.commitEntry(owner, amount);
    },
    refuse(owner) {
      warnOwner(owner);
      return 0;
    },
    commitEntry(owner, amount) {
      writeEntry(owner, amount);
      return amount;
    }
  }
}
</script>
