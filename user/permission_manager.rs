/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */

use resource::constants::server::DEFAULT_USER_NAME;

pub struct PermissionManager {}

impl PermissionManager {
    pub fn exec_user_get_permitted(accessor: &str, subject: &str) -> bool {
        accessor == DEFAULT_USER_NAME || accessor == subject
    }

    pub fn exec_user_all_permitted(accessor: &str) -> bool {
        accessor == DEFAULT_USER_NAME
    }

    pub fn exec_user_create_permitted(accessor: &str) -> bool {
        accessor == DEFAULT_USER_NAME
    }

    pub fn exec_user_update_permitted(accessor: &str, subject: &str) -> bool {
        accessor == DEFAULT_USER_NAME || accessor == subject
    }

    pub fn exec_user_delete_allowed(accessor: &str, subject: &str) -> bool {
        accessor == DEFAULT_USER_NAME || accessor == subject
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ADMIN: &str = DEFAULT_USER_NAME;
    const REGULAR_USER: &str = "alice";
    const OTHER_USER: &str = "bob";

    // --- exec_user_get_permitted ---

    #[test]
    fn admin_can_get_any_user() {
        assert!(PermissionManager::exec_user_get_permitted(ADMIN, REGULAR_USER));
        assert!(PermissionManager::exec_user_get_permitted(ADMIN, OTHER_USER));
        assert!(PermissionManager::exec_user_get_permitted(ADMIN, ADMIN));
    }

    #[test]
    fn user_can_get_self() {
        assert!(PermissionManager::exec_user_get_permitted(REGULAR_USER, REGULAR_USER));
    }

    #[test]
    fn user_cannot_get_other() {
        assert!(!PermissionManager::exec_user_get_permitted(REGULAR_USER, OTHER_USER));
    }

    // --- exec_user_all_permitted ---

    #[test]
    fn admin_can_list_all_users() {
        assert!(PermissionManager::exec_user_all_permitted(ADMIN));
    }

    #[test]
    fn regular_user_cannot_list_all() {
        assert!(!PermissionManager::exec_user_all_permitted(REGULAR_USER));
    }

    // --- exec_user_create_permitted ---

    #[test]
    fn admin_can_create_users() {
        assert!(PermissionManager::exec_user_create_permitted(ADMIN));
    }

    #[test]
    fn regular_user_cannot_create_users() {
        assert!(!PermissionManager::exec_user_create_permitted(REGULAR_USER));
    }

    // --- exec_user_update_permitted ---

    #[test]
    fn admin_can_update_any_user() {
        assert!(PermissionManager::exec_user_update_permitted(ADMIN, REGULAR_USER));
    }

    #[test]
    fn user_can_update_self() {
        assert!(PermissionManager::exec_user_update_permitted(REGULAR_USER, REGULAR_USER));
    }

    #[test]
    fn user_cannot_update_other() {
        assert!(!PermissionManager::exec_user_update_permitted(REGULAR_USER, OTHER_USER));
    }

    // --- exec_user_delete_allowed ---

    #[test]
    fn admin_can_delete_any_user() {
        assert!(PermissionManager::exec_user_delete_allowed(ADMIN, REGULAR_USER));
    }

    #[test]
    fn user_can_delete_self() {
        assert!(PermissionManager::exec_user_delete_allowed(REGULAR_USER, REGULAR_USER));
    }

    #[test]
    fn user_cannot_delete_other() {
        assert!(!PermissionManager::exec_user_delete_allowed(REGULAR_USER, OTHER_USER));
    }
}
