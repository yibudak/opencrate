@{
    Severity = @('Error', 'Warning')
    # CLI build scripts use helper verbs and ShouldProcess is inappropriate for
    # an installer test whose complete install/restore sequence must execute.
    ExcludeRules = @('PSUseApprovedVerbs', 'PSUseShouldProcessForStateChangingFunctions')
}
