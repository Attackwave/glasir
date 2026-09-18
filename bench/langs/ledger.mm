// Keeps the running balance for one account.
#import <Foundation/Foundation.h>

static const NSInteger kLimit = 5000;

@implementation Ledger

// Bills the account and returns what is left.
- (NSInteger)charge:(NSInteger)amount {
    if (amount > kLimit) {
        return [self refuse:amount];
    }
    return [self commitEntry:amount];
}

- (NSInteger)refuse:(NSInteger)amount {
    warnOwner(self.owner);
    return 0;
}

- (NSInteger)commitEntry:(NSInteger)amount {
    writeEntry(self.owner, amount);
    return amount;
}

@end
