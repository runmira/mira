export type AccountType = 'personal' | 'team';

export type ProfileView = {
  fullName: string | null;
  accountType: AccountType | null;
  teamName: string | null;
};

export function useProfile(): { profile: ProfileView | null; loading: boolean } { return { profile: null, loading: false }; }
